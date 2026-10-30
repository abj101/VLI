//! Whisper inference thread: PCM → 16 kHz → rolling buffer → `transcript-update` (Task 4b).

use crate::audio::transcript_event::{
    audio_has_speech, classify_partial, normalize_transcript_candidate, TranscriptPartialKind,
};
use crate::sidecar::{self, Sidecar};
use log::debug;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::SystemTime;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub const DEFAULT_LOCAL_WHISPER_MODEL: &str = "base.en";
pub const LOCAL_WHISPER_MODEL_IDS: &[&str] = &["tiny.en", "base.en", "small.en"];

/// Normalize persisted `local_whisper_model` setting (`tiny.en` | `base.en` | `small.en`).
pub fn parse_local_whisper_model_id(raw: Option<&str>) -> &'static str {
    let s = raw.map(str::trim).unwrap_or("");
    match s {
        "tiny.en" => "tiny.en",
        "base.en" => "base.en",
        "small.en" => "small.en",
        _ => DEFAULT_LOCAL_WHISPER_MODEL,
    }
}

pub(crate) const TARGET_RATE: u32 = 16_000;
/// Ring buffer cap (~4 s at 16 kHz) to bound work per `full` call.
const MAX_BUFFER_SAMPLES: usize = TARGET_RATE as usize * 4;
/// Partial transcript cadence after the first decode.
pub(crate) const INFER_EVERY: Duration = Duration::from_millis(400);
/// First partial decode runs sooner so wake → command feels responsive.
const FIRST_INFER_AFTER: Duration = Duration::from_millis(120);
/// Need some audio before first decode (~100 ms at 16 kHz).
pub(crate) const MIN_DECODE_SAMPLES: usize = TARGET_RATE as usize / 10;
/// Treat chunks below this peak as silence and eventually reset rolling transcript context.
const SILENCE_PEAK_THRESHOLD: f32 = 0.01;
/// Clear stale decode context after this much continuous silence (command HUD).
const SILENCE_RESET_AFTER: Duration = Duration::from_millis(900);
/// Longer silence window while dictating so slow speech does not reset rolling decode.
pub(crate) const SILENCE_RESET_DICTATION: Duration = Duration::from_millis(1800);

/// Whisper `full` thread count: GPU/Vulkan backends already offload heavy ops; keep CPU threads low
/// to avoid oversubscription vs the STT worker thread and system audio.
fn whisper_decode_thread_count(use_accelerator: bool) -> i32 {
    let avail = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(2);
    let cap = if use_accelerator { 2 } else { 4 };
    avail.max(1).min(cap)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptUpdate {
    pub is_final: bool,
    /// Must match the HUD `session_id` when the mic pipeline for this listen was started.
    /// Ignores emissions from detached STT threads after pipeline teardown (`mem::forget` join handle).
    pub hud_session_id: u64,
    pub text: String,
    /// Partial semantics; see [`crate::audio::transcript_event`]. Omitted JSON → `growth`.
    #[serde(default)]
    pub kind: TranscriptPartialKind,
}

/// The server is shared across listen sessions and replaced when model or GPU settings change.
pub struct WhisperServerState(Mutex<Option<WhisperServer>>);

impl Default for WhisperServerState {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}

struct WhisperServer {
    process: Sidecar,
    model_path: PathBuf,
    use_gpu: bool,
}

pub fn clear_whisper_server(app: &AppHandle) {
    if let Some(state) = app.try_state::<WhisperServerState>() {
        if let Ok(mut server) = state.0.lock() {
            *server = None;
        }
    }
}

pub fn resample_mono_to_16k(input: &[f32], input_rate: u32) -> Vec<f32> {
    if input.is_empty() || input_rate == 0 {
        return Vec::new();
    }
    if input_rate == TARGET_RATE {
        return input.to_vec();
    }
    let in_len = input.len() as f64;
    let out_len = ((in_len * TARGET_RATE as f64) / input_rate as f64)
        .floor()
        .max(0.0) as usize;
    if out_len == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(out_len);
    let step = input_rate as f64 / TARGET_RATE as f64;
    for j in 0..out_len {
        let x = j as f64 * step;
        let i = x.floor() as usize;
        let frac = (x - i as f64) as f32;
        let a = *input.get(i).unwrap_or(&0.0);
        let b = *input.get(i.saturating_add(1)).unwrap_or(&a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// Boost quiet mic levels toward ~0.5 peak so Whisper gets usable SNR (WASAPI f32 can sit very low).
/// Does not amplify below [`crate::audio::transcript_event::MIN_SPEECH_PEAK_FOR_DECODE`] — that
/// audio is treated as silence and should not be decoded.
fn normalize_peak_f32(samples: &[f32]) -> Vec<f32> {
    let peak = samples.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
    if peak < crate::audio::transcript_event::MIN_SPEECH_PEAK_FOR_DECODE {
        return samples.to_vec();
    }
    if peak < 1e-8 {
        return samples.to_vec();
    }
    let scale = (0.5_f32 / peak).min(128.0_f32);
    samples
        .iter()
        .map(|s| (s * scale).clamp(-1.0, 1.0))
        .collect()
}

/// Send normalized 16 kHz mono audio to whisper-server's multipart `/inference` endpoint.
fn run_decode(
    app: &AppHandle,
    model_path: &Path,
    audio_16k: &[f32],
    use_gpu: bool,
) -> Result<String, String> {
    if audio_16k.len() < MIN_DECODE_SAMPLES || !audio_has_speech(audio_16k) {
        return Ok(String::new());
    }
    let normalized = normalize_peak_f32(audio_16k);
    let wav = pcm_wav_16k(&normalized);
    let state = app.state::<WhisperServerState>();
    let mut cached = state
        .0
        .lock()
        .map_err(|_| "whisper server state poisoned".to_string())?;
    let needs_restart = cached
        .as_ref()
        .is_some_and(|s| s.model_path != model_path || s.use_gpu != use_gpu);
    if needs_restart {
        *cached = None;
    }
    if cached.is_none() {
        #[cfg(windows)]
        let exe = "whisper/whisper-server.exe";
        #[cfg(target_os = "macos")]
        let exe = "whisper-server";
        #[cfg(not(any(windows, target_os = "macos")))]
        let exe = "whisper-server";
        // b5454 defaults no_context to true; its parser does not accept -nc.
        let mut args = vec![
            "-m".to_string(),
            model_path.to_string_lossy().into_owned(),
            "-l".to_string(),
            "en".to_string(),
            "-t".to_string(),
            whisper_decode_thread_count(use_gpu).to_string(),
        ];
        if !use_gpu {
            args.push("-ng".to_string());
        }
        let process = sidecar::spawn(app, exe, &args, "/health", Duration::from_secs(60))?;
        *cached = Some(WhisperServer {
            process,
            model_path: model_path.to_path_buf(),
            use_gpu,
        });
    }
    let server = cached.as_ref().expect("server initialized");
    let boundary = format!(
        "jarvis-{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let body = inference_multipart(&boundary, &wav);
    let response = ureq::post(&format!(
        "http://127.0.0.1:{}/inference",
        server.process.port
    ))
    .set(
        "Content-Type",
        &format!("multipart/form-data; boundary={boundary}"),
    )
    .timeout(Duration::from_secs(30))
    .send_bytes(&body);
    let response = match response {
        Ok(response) => response,
        Err(e) => {
            *cached = None;
            return Err(format!("whisper inference request failed: {e}"));
        }
    };
    let value: serde_json::Value = serde_json::from_str(
        &response
            .into_string()
            .map_err(|e| format!("read whisper response: {e}"))?,
    )
    .map_err(|e| format!("parse whisper response: {e}"))?;
    value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "whisper response missing text".to_string())
}

fn pcm_wav_16k(samples: &[f32]) -> Vec<u8> {
    let data_len = samples.len().saturating_mul(2).min(u32::MAX as usize - 36) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&16_000u32.to_le_bytes());
    wav.extend_from_slice(&32_000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples.iter().take(data_len as usize / 2) {
        let pcm = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        wav.extend_from_slice(&pcm.to_le_bytes());
    }
    wav
}

fn inference_multipart(boundary: &str, wav: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav.len() + 256);
    for (name, value) in [("response_format", "json"), ("temperature", "0")] {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

fn push_ring(buffer: &mut Vec<f32>, chunk: &[f32]) {
    buffer.extend_from_slice(chunk);
    let excess = buffer.len().saturating_sub(MAX_BUFFER_SAMPLES);
    if excess > 0 {
        buffer.drain(0..excess);
    }
}

#[derive(Debug)]
pub(crate) struct SilenceResetTracker {
    silent_samples: usize,
    reset_after_samples: usize,
}

impl Default for SilenceResetTracker {
    fn default() -> Self {
        Self::for_mode(false)
    }
}

impl SilenceResetTracker {
    pub(crate) fn for_mode(dictation: bool) -> Self {
        let reset_after = if dictation {
            SILENCE_RESET_DICTATION
        } else {
            SILENCE_RESET_AFTER
        };
        let reset_after_samples =
            ((TARGET_RATE as f64) * reset_after.as_secs_f64()).round() as usize;
        Self {
            silent_samples: 0,
            reset_after_samples,
        }
    }

    pub(crate) fn push_and_should_reset(&mut self, chunk_16k: &[f32]) -> bool {
        if chunk_16k.is_empty() {
            return false;
        }
        let peak = chunk_16k
            .iter()
            .fold(0.0f32, |acc, &s| if s.abs() > acc { s.abs() } else { acc });
        if peak <= SILENCE_PEAK_THRESHOLD {
            self.silent_samples = self.silent_samples.saturating_add(chunk_16k.len());
        } else {
            self.silent_samples = 0;
        }

        if self.silent_samples >= self.reset_after_samples {
            self.silent_samples = 0;
            return true;
        }
        false
    }
}

#[expect(clippy::too_many_arguments, reason = "existing STT pipeline API")]
pub fn spawn_stt_thread(
    app: AppHandle,
    model_id: String,
    use_gpu: bool,
    pcm_rx: Receiver<Vec<f32>>,
    input_sample_rate: u32,
    hud_session_id: u64,
    initial_audio_16k: Vec<f32>,
    for_dictation: bool,
    cancel: Arc<AtomicBool>,
) -> JoinHandle<()> {
    let app_err = app.clone();
    std::thread::spawn(move || {
        if cancel.load(Ordering::Relaxed) {
            std::mem::forget(super::spawn_pcm_drain(pcm_rx));
            return;
        }
        let model_path = match crate::audio::whisper_models::ensure_whisper_model(&app, &model_id) {
            Ok(path) => path,
            Err(e) => {
                let _ = app.emit("audio-error", serde_json::json!({ "message": e }));
                std::mem::forget(super::spawn_pcm_drain(pcm_rx));
                return;
            }
        };
        if cancel.load(Ordering::Relaxed) {
            std::mem::forget(super::spawn_pcm_drain(pcm_rx));
            return;
        }
        if let Err(e) = stt_loop(
            app.clone(),
            model_path,
            use_gpu,
            pcm_rx,
            input_sample_rate,
            hud_session_id,
            initial_audio_16k,
            for_dictation,
            cancel,
        ) {
            let _ = app_err.emit("audio-error", serde_json::json!({ "message": e }));
        }
    })
}

#[expect(clippy::too_many_arguments, reason = "existing STT pipeline API")]
fn stt_loop(
    app: AppHandle,
    model_path: PathBuf,
    use_gpu: bool,
    pcm_rx: Receiver<Vec<f32>>,
    input_sample_rate: u32,
    hud_session_id: u64,
    initial_audio_16k: Vec<f32>,
    for_dictation: bool,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let had_preroll = !initial_audio_16k.is_empty();
    let mut buffer_16k: Vec<f32> = initial_audio_16k;
    let mut last_decode = Instant::now();
    let mut last_text = String::new();
    let mut silence_reset = SilenceResetTracker::for_mode(for_dictation);
    let mut first_decode_pending = true;
    if had_preroll {
        last_decode = Instant::now() - FIRST_INFER_AFTER;
    }

    let maybe_decode = |buffer_16k: &mut Vec<f32>,
                        last_decode: &mut Instant,
                        last_text: &mut String,
                        first_decode_pending: &mut bool|
     -> Result<(), String> {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let cadence = if *first_decode_pending {
            FIRST_INFER_AFTER
        } else {
            INFER_EVERY
        };
        if last_decode.elapsed() < cadence {
            return Ok(());
        }
        *last_decode = Instant::now();
        *first_decode_pending = false;

        let text = match run_decode(&app, &model_path, buffer_16k, use_gpu) {
            Ok(t) => t,
            Err(e) => {
                let _ = app.emit(
                    "audio-error",
                    serde_json::json!({ "message": format!("whisper decode failed: {e}") }),
                );
                return Ok(());
            }
        };
        let text = normalize_transcript_candidate(&text);

        if let Some(text) = text {
            let Some(kind) = classify_partial(last_text, &text) else {
                return Ok(());
            };
            *last_text = text.clone();
            debug!(
                "stt: emit transcript-update partial kind={kind:?} chars={} preview={:?}",
                text.chars().count(),
                text.chars().take(48).collect::<String>()
            );
            let _ = app.emit(
                "transcript-update",
                TranscriptUpdate {
                    text,
                    is_final: false,
                    hud_session_id,
                    kind,
                },
            );
        } else if !last_text.is_empty() {
            last_text.clear();
            debug!("stt: suppress non-word/silence decode; clear transcript");
            let _ = app.emit(
                "transcript-update",
                TranscriptUpdate {
                    text: String::new(),
                    is_final: false,
                    hud_session_id,
                    kind: TranscriptPartialKind::SilenceReset,
                },
            );
        }
        Ok(())
    };

    if !buffer_16k.is_empty() && !cancel.load(Ordering::Relaxed) {
        maybe_decode(
            &mut buffer_16k,
            &mut last_decode,
            &mut last_text,
            &mut first_decode_pending,
        )?;
    }

    while let Ok(chunk) = pcm_rx.recv() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let chunk_16k = resample_mono_to_16k(&chunk, input_sample_rate);
        if silence_reset.push_and_should_reset(&chunk_16k) {
            debug!("stt: silence gap reached; reset rolling decode context");
            buffer_16k.clear();
            if !last_text.is_empty() {
                last_text.clear();
                let _ = app.emit(
                    "transcript-update",
                    TranscriptUpdate {
                        text: String::new(),
                        is_final: false,
                        hud_session_id,
                        kind: TranscriptPartialKind::SilenceReset,
                    },
                );
            } else {
                last_text.clear();
            }
            first_decode_pending = true;
            last_decode = Instant::now();
            continue;
        }
        push_ring(&mut buffer_16k, &chunk_16k);
        maybe_decode(
            &mut buffer_16k,
            &mut last_decode,
            &mut last_text,
            &mut first_decode_pending,
        )?;
    }

    // Channel closed: final pass (best effort) unless this session was cancelled.
    if !cancel.load(Ordering::Relaxed) && buffer_16k.len() >= MIN_DECODE_SAMPLES {
        if let Ok(text) = run_decode(&app, &model_path, &buffer_16k, use_gpu) {
            if let Some(text) = normalize_transcript_candidate(&text) {
                debug!(
                    "stt: emit transcript-update final chars={} preview={:?}",
                    text.chars().count(),
                    text.chars().take(48).collect::<String>()
                );
                let _ = app.emit(
                    "transcript-update",
                    TranscriptUpdate {
                        text,
                        is_final: true,
                        hud_session_id,
                        kind: TranscriptPartialKind::Growth,
                    },
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        inference_multipart, normalize_peak_f32, parse_local_whisper_model_id, pcm_wav_16k,
        resample_mono_to_16k, SilenceResetTracker, TranscriptPartialKind, TranscriptUpdate,
        TARGET_RATE,
    };
    use crate::audio::whisper_models::whisper_model_filename;

    #[test]
    fn parse_local_whisper_model_id_normalizes() {
        assert_eq!(parse_local_whisper_model_id(None), "base.en");
        assert_eq!(parse_local_whisper_model_id(Some("base.en")), "base.en");
        assert_eq!(parse_local_whisper_model_id(Some("bogus")), "base.en");
    }

    #[test]
    fn whisper_model_filename_maps_ids() {
        assert_eq!(whisper_model_filename("base.en"), "ggml-base.en.bin");
    }

    #[test]
    fn resample_identity_16k() {
        let v = vec![0.25f32, -0.5, 1.0];
        let out = resample_mono_to_16k(&v, 16_000);
        assert_eq!(out, v);
    }

    #[test]
    fn resample_48k_to_16k_length() {
        let v: Vec<f32> = (0..48).map(|i| i as f32 / 48.0).collect();
        let out = resample_mono_to_16k(&v, 48_000);
        assert_eq!(out.len(), 16);
    }

    #[test]
    fn normalize_peak_boosts_quiet_speech() {
        // Above MIN_SPEECH_PEAK_FOR_DECODE (0.02) but still quiet — should boost toward 0.5.
        let v = vec![0.03f32, -0.03];
        let n = normalize_peak_f32(&v);
        assert!((n[0] - 0.5).abs() < 0.05);
        assert!((n[1] + 0.5).abs() < 0.05);
    }

    #[test]
    fn transcript_update_json_shape() {
        let u = TranscriptUpdate {
            text: "hello".into(),
            is_final: true,
            hud_session_id: 42,
            kind: TranscriptPartialKind::Growth,
        };
        let j = serde_json::to_value(&u).expect("serialize");
        assert_eq!(j["text"], "hello");
        assert_eq!(j["is_final"], true);
        assert_eq!(j["hud_session_id"], 42);
        assert_eq!(j["kind"], "growth");
    }

    #[test]
    fn transcript_update_kind_defaults_when_omitted() {
        let v: TranscriptUpdate =
            serde_json::from_str(r#"{"text":"hi","is_final":false,"hud_session_id":1}"#)
                .expect("deserialize");
        assert_eq!(v.kind, TranscriptPartialKind::Growth);
    }

    #[test]
    fn pcm_wav_header_and_samples_are_mono_16khz_pcm16() {
        let wav = pcm_wav_16k(&[0.5, -1.0]);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 40);
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 4);
        assert_eq!(i16::from_le_bytes(wav[44..46].try_into().unwrap()), 16_384);
        assert_eq!(
            i16::from_le_bytes(wav[46..48].try_into().unwrap()),
            i16::MIN + 1
        );
    }

    #[test]
    fn multipart_contains_json_temperature_and_audio_file() {
        let body = String::from_utf8(inference_multipart("boundary-test", b"RIFF")).unwrap();
        assert!(body.contains("name=\"response_format\"\r\n\r\njson"));
        assert!(body.contains("name=\"temperature\"\r\n\r\n0"));
        assert!(body.contains("name=\"file\"; filename=\"audio.wav\""));
        assert!(body.contains("Content-Type: audio/wav\r\n\r\nRIFF"));
        assert!(body.ends_with("\r\n--boundary-test--\r\n"));
    }

    #[test]
    fn normalize_peak_does_not_boost_sub_speech_floor() {
        let quiet = vec![0.005f32; 100];
        assert_eq!(normalize_peak_f32(&quiet), quiet);
    }

    #[test]
    fn silence_tracker_resets_after_long_silence() {
        let mut tracker = SilenceResetTracker::for_mode(false);
        let silence = vec![0.0f32; TARGET_RATE as usize / 2];
        assert!(!tracker.push_and_should_reset(&silence));
        assert!(tracker.push_and_should_reset(&silence));
    }
}

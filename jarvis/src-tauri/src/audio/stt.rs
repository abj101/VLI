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
pub const LOCAL_WHISPER_MODEL_IDS: &[&str] =
    &["tiny.en", "base.en", "small.en", "large-v3-turbo-q5_0"];

/// Normalize persisted `local_whisper_model` setting (`tiny.en` | `base.en` | `small.en`).
pub fn parse_local_whisper_model_id(raw: Option<&str>) -> &'static str {
    let s = raw.map(str::trim).unwrap_or("");
    match s {
        "tiny.en" => "tiny.en",
        "base.en" => "base.en",
        "small.en" => "small.en",
        "large-v3-turbo-q5_0" => "large-v3-turbo-q5_0",
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
pub(crate) const SILENCE_RESET_DICTATION: Duration = Duration::from_millis(700);

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
    run_decode_prompt(app, model_path, audio_16k, use_gpu, &vocabulary_prompt(app))
}

fn run_decode_prompt(
    app: &AppHandle,
    model_path: &Path,
    audio_16k: &[f32],
    use_gpu: bool,
    prompt: &str,
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
    let port = ensure_whisper_server(app, &mut cached, model_path, use_gpu)?;
    let decode_started = Instant::now();
    let text = match post_inference(port, &wav, prompt) {
        Ok(text) => text,
        Err(e) => {
            *cached = None;
            return Err(e);
        }
    };
    debug!(
        "stt: HTTP decode completed in {} ms ({} samples)",
        decode_started.elapsed().as_millis(),
        audio_16k.len()
    );
    Ok(text)
}

/// Spawn whisper-server and run one throwaway decode so the first real listen is warm.
pub fn warm_whisper_server(app: &AppHandle, model_id: &str, use_gpu: bool) {
    let started = Instant::now();
    let result =
        crate::audio::whisper_models::ensure_whisper_model(app, model_id).and_then(|path| {
            let state = app.state::<WhisperServerState>();
            let mut cached = state
                .0
                .lock()
                .map_err(|_| "whisper server state poisoned".to_string())?;
            let port = ensure_whisper_server(app, &mut cached, &path, use_gpu)?;
            post_inference(port, &pcm_wav_16k(&vec![0.0; TARGET_RATE as usize]), "")
        });
    match result {
        Ok(_) => log::info!("stt: whisper warm in {} ms", started.elapsed().as_millis()),
        Err(e) => log::warn!("stt: whisper warmup failed: {e}"),
    }
}

/// Whisper's initial prompt biases spelling toward these words ("Claude", not "cloud").
/// Short installed-app names, since those are what people say out loud.
fn vocabulary_prompt(app: &AppHandle) -> String {
    let mut names: Vec<String> = app
        .try_state::<crate::AppIndexStore>()
        .and_then(|s| {
            s.read()
                .ok()
                .map(|g| g.iter().map(|e| e.display_name.clone()).collect())
        })
        .unwrap_or_default();
    names.retain(|n| {
        n.split_whitespace().count() <= 2
            && n.chars().all(|c| c.is_alphabetic() || c == ' ')
            && !n.to_lowercase().contains("uninstall")
    });
    names.sort_by_key(|n| (n.len(), n.to_lowercase()));
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    // ponytail: shortest names first, capped well under Whisper's ~224-token prompt window.
    let mut prompt = String::from("Hey Jarvis, open Claude");
    for name in names {
        if prompt.len() + name.len() > 600 {
            break;
        }
        prompt.push_str(", ");
        prompt.push_str(&name);
    }
    prompt.push('.');
    prompt
}

fn ensure_whisper_server(
    app: &AppHandle,
    cached: &mut Option<WhisperServer>,
    model_path: &Path,
    use_gpu: bool,
) -> Result<u16, String> {
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
    Ok(cached.as_ref().expect("server initialized").process.port)
}

fn post_inference(port: u16, wav: &[u8], prompt: &str) -> Result<String, String> {
    let boundary = format!(
        "jarvis-{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let body = inference_multipart(&boundary, wav, prompt);
    let response = ureq::post(&format!("http://127.0.0.1:{port}/inference"))
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .timeout(Duration::from_secs(30))
        .send_bytes(&body)
        .map_err(|e| format!("whisper inference request failed: {e}"))?;
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

fn inference_multipart(boundary: &str, wav: &[u8], prompt: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav.len() + prompt.len() + 256);
    let fields = [
        ("response_format", "json"),
        ("temperature", "0"),
        ("prompt", prompt),
    ];
    for (name, value) in fields.into_iter().filter(|(_, v)| !v.is_empty()) {
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
    if for_dictation {
        return dictation_loop(
            &app,
            &model_path,
            use_gpu,
            pcm_rx,
            input_sample_rate,
            hud_session_id,
            cancel,
        );
    }
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

    while let Ok(first) = pcm_rx.recv() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        // Decoding blocks this loop, so take every chunk queued meanwhile before the next one.
        // Otherwise a decode slower than `INFER_EVERY` runs once per ~10 ms chunk and the
        // transcript falls further behind live speech on every pass (looks frozen).
        let queued = std::iter::from_fn(|| pcm_rx.try_recv().ok());
        for chunk in std::iter::once(first).chain(queued) {
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
                }
                first_decode_pending = true;
                last_decode = Instant::now();
                continue;
            }
            push_ring(&mut buffer_16k, &chunk_16k);
        }
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

pub(crate) fn trim_utterance(samples: &[f32]) -> &[f32] {
    let frame = TARGET_RATE as usize / 50;
    let threshold = crate::audio::transcript_event::MIN_SPEECH_PEAK_FOR_DECODE;
    let speech = |chunk: &[f32]| chunk.iter().any(|s| s.abs() >= threshold);
    let Some(first) = samples.chunks(frame).position(speech) else {
        return &[];
    };
    let last = samples.chunks(frame).rposition(speech).unwrap_or(first);
    let padding = TARGET_RATE as usize * 150 / 1000;
    let start = (first * frame).saturating_sub(padding);
    let end = ((last + 1) * frame)
        .saturating_add(padding)
        .min(samples.len());
    &samples[start..end]
}

enum DictationJob {
    /// The utterance so far, for a live hypothesis.
    Partial(Vec<f32>),
    /// The finished utterance.
    Final(Vec<f32>),
}

/// Audio side of live dictation: never decodes itself, so speech can't back up. It hands the
/// growing utterance to `decode_dictation` every `INFER_EVERY`, and the finished one at a pause.
fn dictation_loop(
    app: &AppHandle,
    model: &Path,
    gpu: bool,
    rx: Receiver<Vec<f32>>,
    rate: u32,
    id: u64,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut utterances = Utterances::default();
    let mut tail = DictationTail::for_app(app);
    let end_on_pause = crate::load_dictation_hotkey_mode(app) == crate::DictationHotkeyMode::Toggle;
    let (jobs, job_rx) = std::sync::mpsc::channel();
    let decoder = {
        let (app, model, cancel) = (app.clone(), model.to_path_buf(), Arc::clone(&cancel));
        std::thread::spawn(move || decode_dictation(&app, &model, gpu, id, job_rx, &cancel))
    };
    let mut last_partial = Instant::now();
    while let Ok(chunk) = rx.recv() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let ready = tail.push(&resample_mono_to_16k(&chunk, rate));
        if let Some(utterance) = utterances.push(&ready, end_on_pause) {
            let _ = jobs.send(DictationJob::Final(utterance));
            last_partial = Instant::now();
        } else if last_partial.elapsed() >= INFER_EVERY {
            if let Some(live) = utterances.live() {
                let _ = jobs.send(DictationJob::Partial(live.to_vec()));
                last_partial = Instant::now();
            }
        }
    }
    if !cancel.load(Ordering::Relaxed) {
        if let Some(utterance) = utterances.push(
            &tail.finish(crate::dictation::stopped_by_snap(app, id)),
            end_on_pause,
        ) {
            let _ = jobs.send(DictationJob::Final(utterance));
        }
        if let Some(utterance) = utterances.finish() {
            let _ = jobs.send(DictationJob::Final(utterance));
        }
    }
    drop(jobs);
    // Stop waits for the last words to land.
    let _ = decoder.join();
    Ok(())
}

/// Decoder side: types words as soon as two consecutive live hypotheses agree on them, then the
/// rest when the utterance ends. Only the newest partial is decoded, so it never falls behind.
/// Utterances that end before anything is committed go through the normal path (with cleanup).
fn decode_dictation(
    app: &AppHandle,
    model: &Path,
    gpu: bool,
    id: u64,
    jobs: Receiver<DictationJob>,
    cancel: &AtomicBool,
) {
    let mut prev = String::new();
    let mut committed = 0usize;
    let mut prompt = None::<String>;
    while let Ok(mut job) = jobs.recv() {
        // A newer partial supersedes this one; a final always wins.
        while matches!(job, DictationJob::Partial(_)) {
            match jobs.try_recv() {
                Ok(next) => job = next,
                Err(_) => break,
            }
        }
        if cancel.load(Ordering::Relaxed) {
            continue;
        }
        // Prior utterances only: this utterance's own words must not be in its prompt.
        let prompt_text = prompt
            .get_or_insert_with(|| crate::dictation::dictation_prompt(app, id))
            .clone();
        let decode = |audio: &[f32]| {
            run_decode_prompt(app, model, trim_utterance(audio), gpu, &prompt_text)
                .ok()
                .and_then(|raw| normalize_transcript_candidate(&raw))
        };
        match job {
            DictationJob::Partial(audio) => {
                let next = decode(&audio).unwrap_or_default();
                let words = agreed_words(&prev, &next, committed);
                prev = next;
                if !words.is_empty() {
                    committed += words.len();
                    let _ = crate::dictation::commit_live(app, id, &words.join(" "));
                }
            }
            DictationJob::Final(audio) => {
                let started = Instant::now();
                if let Some(text) = decode(&audio) {
                    if committed == 0 {
                        let _ = crate::dictation::publish_transcript(
                            app,
                            TranscriptUpdate {
                                text,
                                is_final: true,
                                hud_session_id: id,
                                kind: TranscriptPartialKind::Growth,
                            },
                            started.elapsed().as_millis(),
                        );
                    } else {
                        // ponytail: assumes the final keeps the committed words as its prefix;
                        // Whisper rarely rewrites earlier words, at worst a word is doubled or lost.
                        let rest: Vec<&str> = text.split_whitespace().skip(committed).collect();
                        if !rest.is_empty() {
                            let _ = crate::dictation::commit_live(app, id, &rest.join(" "));
                        }
                    }
                }
                let _ = crate::dictation::finish_live_utterance(app, id);
                prev.clear();
                committed = 0;
                prompt = None;
            }
        }
    }
}

/// LocalAgreement: words two consecutive hypotheses share (ignoring case and punctuation),
/// except the newest's last word (it may be cut mid-word), past the `committed` count.
pub(crate) fn agreed_words(prev: &str, next: &str, committed: usize) -> Vec<String> {
    let norm = |w: &str| {
        w.chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };
    let next_words: Vec<&str> = next.split_whitespace().collect();
    let common = prev
        .split_whitespace()
        .zip(&next_words)
        .take_while(|(a, b)| norm(a) == norm(b))
        .count();
    let stable = common.min(next_words.len().saturating_sub(1));
    next_words
        .get(committed..stable)
        .map(|w| w.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

/// Keep the snap confirmation window out of every decode, including live partials.
pub(crate) struct DictationTail {
    samples: Vec<f32>,
    keep: usize,
}

impl DictationTail {
    pub(crate) fn for_app(app: &AppHandle) -> Self {
        let enabled = crate::open_db_connection(app)
            .ok()
            .and_then(|conn| crate::db::get_app_settings(&conn).ok())
            .is_some_and(|s| s.snap_toggle_enabled && s.wake_engine == "oww");
        Self {
            samples: Vec::new(),
            keep: if enabled { TARGET_RATE as usize / 4 } else { 0 },
        }
    }

    pub(crate) fn push(&mut self, chunk: &[f32]) -> Vec<f32> {
        self.samples.extend_from_slice(chunk);
        let ready = self.samples.len().saturating_sub(self.keep);
        self.samples.drain(..ready).collect()
    }

    pub(crate) fn finish(self, discard: bool) -> Vec<f32> {
        if discard {
            Vec::new()
        } else {
            self.samples
        }
    }
}

/// Splits dictation audio into utterances. Speech needs a sustained voiced run, so key clicks
/// (deleting, typing) never reach Whisper, and the pause floor follows the speaker's level so
/// fan or room hum above the fixed floor still ends the sentence.
#[derive(Default)]
pub(crate) struct Utterances {
    buffer: Vec<f32>,
    voiced_run: usize,
    speech: bool,
    peak: f32,
    silent: usize,
}

impl Utterances {
    const MIN_VOICED: usize = TARGET_RATE as usize * 120 / 1000;
    const CAP: usize = TARGET_RATE as usize * 28;
    const IDLE_KEEP: usize = TARGET_RATE as usize;

    /// Feed 16 kHz audio; returns a finished utterance (speech then a pause, or the 28 s cap).
    pub(crate) fn push(&mut self, chunk: &[f32], end_on_pause: bool) -> Option<Vec<f32>> {
        self.buffer.extend_from_slice(chunk);
        for frame in chunk.chunks(TARGET_RATE as usize / 50) {
            let p = crate::audio::transcript_event::audio_peak(frame);
            if p > crate::audio::transcript_event::MIN_SPEECH_PEAK_FOR_DECODE {
                self.voiced_run += frame.len();
                if self.voiced_run >= Self::MIN_VOICED {
                    self.speech = true;
                    self.peak = self.peak.max(p);
                }
            } else {
                self.voiced_run = 0;
            }
            if p <= SILENCE_PEAK_THRESHOLD.max(self.peak * 0.15) {
                self.silent += frame.len();
            } else {
                self.silent = 0;
            }
        }
        let paused =
            self.silent as f64 >= TARGET_RATE as f64 * SILENCE_RESET_DICTATION.as_secs_f64();
        if (end_on_pause && self.speech && paused) || self.buffer.len() >= Self::CAP {
            return self.finish_and_reset();
        }
        if !self.speech && self.buffer.len() > 2 * Self::IDLE_KEEP {
            // Idle noise never piles up; keep a second so the first syllable survives.
            self.buffer.drain(..self.buffer.len() - Self::IDLE_KEEP);
        }
        None
    }

    /// The utterance so far, once it holds speech.
    pub(crate) fn live(&self) -> Option<&[f32]> {
        self.speech.then_some(&self.buffer[..])
    }

    pub(crate) fn finish(mut self) -> Option<Vec<f32>> {
        self.finish_and_reset()
    }

    fn finish_and_reset(&mut self) -> Option<Vec<f32>> {
        let had_speech = self.speech;
        let buffer = std::mem::take(self).buffer;
        had_speech.then_some(buffer)
    }
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
    fn dictation_tail_holds_snap_audio_and_preserves_normal_stop() {
        let make_tail = || super::DictationTail {
            samples: Vec::new(),
            keep: 4_000,
        };
        let mut tail = make_tail();
        let speech = vec![0.2; 8_000];
        let snap = vec![0.9; 4_000];
        let mut decoded = tail.push(&speech);
        decoded.extend(tail.push(&snap));
        assert_eq!(decoded, speech);
        assert!(tail.finish(true).is_empty());

        let mut tail = make_tail();
        assert!(tail.push(&speech[..2_000]).is_empty());
        assert_eq!(tail.finish(false), speech[..2_000]);
        let mut disabled = super::DictationTail {
            samples: Vec::new(),
            keep: 0,
        };
        assert_eq!(disabled.push(&speech), speech);
        assert!(disabled.finish(false).is_empty());
    }

    #[test]
    fn agreed_words_commit_stable_prefix_only() {
        use super::agreed_words;
        assert!(agreed_words("", "hello there", 0).is_empty());
        // "hello" agrees and isn't last; "there" is still the trailing word.
        assert_eq!(agreed_words("Hello the", "hello, there", 0), vec!["hello,"]);
        assert_eq!(
            agreed_words("hello there my", "Hello there, my friend", 1),
            vec!["there,", "my"]
        );
        // A revision before the committed point commits nothing new.
        assert!(agreed_words("yellow there", "hello there now", 1).is_empty());
    }

    #[test]
    fn utterances_ignore_key_clicks_and_end_on_hum_level_pause() {
        let ms = |n: usize, level: f32| vec![level; TARGET_RATE as usize * n / 1000];
        let mut u = super::Utterances::default();
        // 20 key clicks (30 ms spikes) over quiet: never an utterance, buffer stays bounded.
        for _ in 0..20 {
            assert!(u.push(&ms(30, 0.5), true).is_none());
            assert!(u.push(&ms(300, 0.0), true).is_none());
        }
        assert!(u.buffer.len() <= 2 * TARGET_RATE as usize);
        // Speech, then a pause at fan-hum level (above the fixed 0.01 floor) ends it.
        assert!(u.push(&ms(600, 0.3), true).is_none());
        let utterance = u
            .push(&ms(800, 0.015), true)
            .expect("pause ends the utterance");
        assert!(utterance.len() >= TARGET_RATE as usize * 1400 / 1000);
        assert!(u.finish().is_none());
    }

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
        let body = String::from_utf8(inference_multipart(
            "boundary-test",
            b"RIFF",
            "Hey Jarvis, open Claude.",
        ))
        .unwrap();
        assert!(body.contains("name=\"prompt\"\r\n\r\nHey Jarvis, open Claude."));
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

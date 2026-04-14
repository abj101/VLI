//! Whisper inference thread: PCM → 16 kHz → rolling buffer → `transcript-update` (Task 4b).

use log::debug;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext};

const WHISPER_MODEL_FILE: &str = "ggml-tiny.en.bin";
const TARGET_RATE: u32 = 16_000;
/// Ring buffer cap (~4 s at 16 kHz) to bound work per `full` call.
const MAX_BUFFER_SAMPLES: usize = TARGET_RATE as usize * 4;
/// Partial transcript cadence.
const INFER_EVERY: Duration = Duration::from_millis(750);
/// Need some audio before first decode.
const MIN_DECODE_SAMPLES: usize = TARGET_RATE as usize / 4;
/// Treat chunks below this peak as silence and eventually reset rolling transcript context.
const SILENCE_PEAK_THRESHOLD: f32 = 0.01;
/// Clear stale decode context after this much continuous silence.
const SILENCE_RESET_AFTER: Duration = Duration::from_millis(900);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptUpdate {
    pub text: String,
    pub is_final: bool,
    /// Must match the HUD `session_id` when the mic pipeline for this listen was started.
    /// Ignores emissions from detached STT threads after pipeline teardown (`mem::forget` join handle).
    pub hud_session_id: u64,
}

/// Tauri `resource_dir()` may not match `src-tauri/resources/` during `tauri dev`; keep a crate-relative fallback.
fn whisper_model_candidates(app: &AppHandle) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(dir) = app.path().resource_dir() {
        out.push(dir.join(WHISPER_MODEL_FILE));
    }
    out.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(WHISPER_MODEL_FILE),
    );
    out
}

pub fn resolve_whisper_model_path(app: &AppHandle) -> Result<PathBuf, String> {
    let candidates = whisper_model_candidates(app);
    for path in &candidates {
        if path.is_file() {
            return Ok(path.clone());
        }
    }
    let tried = candidates
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "Whisper model `{}` not found (tried: {}). Run `scripts/download-model.ps1` from the jarvis folder.",
        WHISPER_MODEL_FILE, tried
    ))
}

/// Linear resample mono `f32` to 16 kHz (Whisper input).
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
fn normalize_peak_f32(samples: &[f32]) -> Vec<f32> {
    let peak = samples.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
    if peak < 1e-8 {
        return samples.to_vec();
    }
    let scale = (0.5_f32 / peak).min(128.0_f32);
    samples
        .iter()
        .map(|s| (s * scale).clamp(-1.0, 1.0))
        .collect()
}

/// One-shot decode: fresh [`whisper_rs::WhisperState`] per call so sliding-window passes do not
/// reuse stale KV / prompt state (see `FullParams::set_no_context` / `set_single_segment`).
fn run_decode(ctx: &WhisperContext, audio_16k: &[f32]) -> Result<String, whisper_rs::WhisperError> {
    if audio_16k.len() < MIN_DECODE_SAMPLES {
        return Ok(String::new());
    }
    let audio = normalize_peak_f32(audio_16k);
    let mut state = ctx.create_state()?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_no_context(true);
    params.set_single_segment(true);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(4) as i32)
        .unwrap_or(2);
    params.set_n_threads(threads);
    state.full(params, &audio)?;
    let n = state.full_n_segments()?;
    let mut s = String::new();
    for i in 0..n {
        let seg = state.full_get_segment_text(i)?;
        let t = seg.trim();
        if t.is_empty() {
            continue;
        }
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(t);
    }
    Ok(s)
}

fn push_ring(buffer: &mut Vec<f32>, chunk: &[f32]) {
    buffer.extend_from_slice(chunk);
    let excess = buffer.len().saturating_sub(MAX_BUFFER_SAMPLES);
    if excess > 0 {
        buffer.drain(0..excess);
    }
}

fn is_word_like_token(token: &str) -> bool {
    let mut letters = 0usize;
    let mut digits = 0usize;
    let mut total = 0usize;
    for ch in token.chars() {
        if ch.is_whitespace() {
            continue;
        }
        total += 1;
        if ch.is_alphabetic() {
            letters += 1;
        } else if ch.is_ascii_digit() {
            digits += 1;
        }
    }
    if total == 0 {
        return false;
    }
    letters >= 2 || (letters >= 1 && digits >= 1 && total <= 8)
}

/// Keep only plausible word output from Whisper and normalize whitespace.
/// Returns `None` for silence/noise-like decode output.
fn normalize_transcript_candidate(raw: &str) -> Option<String> {
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    let has_word_like = normalized.split_whitespace().any(is_word_like_token);
    if !has_word_like {
        return None;
    }
    Some(normalized)
}

#[derive(Debug, Default)]
struct SilenceResetTracker {
    silent_samples: usize,
}

impl SilenceResetTracker {
    fn new() -> Self {
        Self::default()
    }

    fn push_and_should_reset(&mut self, chunk_16k: &[f32]) -> bool {
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

        let reset_after_samples =
            ((TARGET_RATE as f64) * SILENCE_RESET_AFTER.as_secs_f64()).round() as usize;
        if self.silent_samples >= reset_after_samples {
            self.silent_samples = 0;
            return true;
        }
        false
    }
}

pub fn spawn_stt_thread(
    app: AppHandle,
    ctx: WhisperContext,
    pcm_rx: Receiver<Vec<f32>>,
    input_sample_rate: u32,
    hud_session_id: u64,
) -> JoinHandle<()> {
    let app_err = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = stt_loop(app, ctx, pcm_rx, input_sample_rate, hud_session_id) {
            let _ = app_err.emit("audio-error", serde_json::json!({ "message": e }));
        }
    })
}

fn stt_loop(
    app: AppHandle,
    ctx: WhisperContext,
    pcm_rx: Receiver<Vec<f32>>,
    input_sample_rate: u32,
    hud_session_id: u64,
) -> Result<(), String> {
    let mut buffer_16k: Vec<f32> = Vec::new();
    let mut last_decode = Instant::now() - INFER_EVERY;
    let mut last_text = String::new();
    let mut silence_reset = SilenceResetTracker::new();

    while let Ok(chunk) = pcm_rx.recv() {
        let chunk_16k = resample_mono_to_16k(&chunk, input_sample_rate);
        if silence_reset.push_and_should_reset(&chunk_16k) {
            debug!("stt: silence gap reached; reset rolling decode context");
            buffer_16k.clear();
            last_text.clear();
            continue;
        }
        push_ring(&mut buffer_16k, &chunk_16k);

        if last_decode.elapsed() < INFER_EVERY {
            continue;
        }
        last_decode = Instant::now();

        let text = match run_decode(&ctx, &buffer_16k) {
            Ok(t) => t,
            Err(e) => {
                let _ = app.emit(
                    "audio-error",
                    serde_json::json!({ "message": format!("whisper decode failed: {e}") }),
                );
                continue;
            }
        };
        let text = normalize_transcript_candidate(&text);

        if let Some(text) = text {
            if text == last_text {
                continue;
            }
            last_text = text.clone();
            debug!(
                "stt: emit transcript-update partial chars={} preview={:?}",
                text.chars().count(),
                text.chars().take(48).collect::<String>()
            );
            let _ = app.emit(
                "transcript-update",
                TranscriptUpdate {
                    text,
                    is_final: false,
                    hud_session_id,
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
                },
            );
        }
    }

    // Channel closed: final pass (best effort).
    if buffer_16k.len() >= MIN_DECODE_SAMPLES {
        if let Ok(text) = run_decode(&ctx, &buffer_16k) {
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
                    },
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        normalize_peak_f32, normalize_transcript_candidate, resample_mono_to_16k,
        SilenceResetTracker, TranscriptUpdate, TARGET_RATE,
    };

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
    fn normalize_peak_boosts_quiet_audio() {
        let v = vec![0.01f32, -0.01];
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
        };
        let j = serde_json::to_value(&u).expect("serialize");
        assert_eq!(j["text"], "hello");
        assert_eq!(j["is_final"], true);
        assert_eq!(j["hud_session_id"], 42);
    }

    #[test]
    fn manifest_dir_resources_points_at_bundled_filename() {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(super::WHISPER_MODEL_FILE);
        assert!(p.ends_with(super::WHISPER_MODEL_FILE));
    }

    #[test]
    fn silence_tracker_resets_after_long_silence() {
        let mut t = SilenceResetTracker::new();
        let silence = vec![0.0f32; (TARGET_RATE as usize) / 2];
        assert!(!t.push_and_should_reset(&silence));
        assert!(t.push_and_should_reset(&silence));
    }

    #[test]
    fn silence_tracker_clears_after_loud_chunk() {
        let mut t = SilenceResetTracker::new();
        let silence = vec![0.0f32; (TARGET_RATE as usize) / 2];
        let loud = vec![0.5f32; (TARGET_RATE as usize) / 20];
        assert!(!t.push_and_should_reset(&silence));
        assert!(!t.push_and_should_reset(&loud));
        assert!(!t.push_and_should_reset(&silence));
        assert!(t.push_and_should_reset(&silence));
    }

    #[test]
    fn transcript_candidate_rejects_noise_only_text() {
        assert_eq!(normalize_transcript_candidate("... --- !!!"), None);
        assert_eq!(normalize_transcript_candidate("   "), None);
    }

    #[test]
    fn transcript_candidate_accepts_word_like_text() {
        assert_eq!(
            normalize_transcript_candidate("  open   notepad now  "),
            Some("open notepad now".into())
        );
    }
}

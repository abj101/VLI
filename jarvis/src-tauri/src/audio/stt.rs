//! Whisper inference thread: PCM → 16 kHz → rolling buffer → `transcript-update` (Task 4b).

use serde::Serialize;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

const WHISPER_MODEL_FILE: &str = "ggml-tiny.en.bin";
const TARGET_RATE: u32 = 16_000;
/// Ring buffer cap (~4 s at 16 kHz) to bound work per `full` call.
const MAX_BUFFER_SAMPLES: usize = TARGET_RATE as usize * 4;
/// Partial transcript cadence.
const INFER_EVERY: Duration = Duration::from_millis(750);
/// Need some audio before first decode.
const MIN_DECODE_SAMPLES: usize = TARGET_RATE as usize / 4;

#[derive(Debug, Clone, Serialize)]
pub struct TranscriptUpdate {
    pub text: String,
    pub is_final: bool,
}

pub fn resolve_whisper_model_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?;
    let path = dir.join(WHISPER_MODEL_FILE);
    if !path.is_file() {
        return Err(format!(
            "Whisper model missing at `{}` (run `scripts/download-model.ps1` from the jarvis folder)",
            path.display()
        ));
    }
    Ok(path)
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
    let out_len = ((in_len * TARGET_RATE as f64) / input_rate as f64).floor().max(0.0) as usize;
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

fn run_decode(state: &mut WhisperState, audio_16k: &[f32]) -> Result<String, whisper_rs::WhisperError> {
    if audio_16k.len() < MIN_DECODE_SAMPLES {
        return Ok(String::new());
    }
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(4) as i32)
        .unwrap_or(2);
    params.set_n_threads(threads);
    state.full(params, audio_16k)?;
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

pub fn spawn_stt_thread(
    app: AppHandle,
    ctx: WhisperContext,
    pcm_rx: Receiver<Vec<f32>>,
    input_sample_rate: u32,
) -> JoinHandle<()> {
    let app_err = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = stt_loop(app, ctx, pcm_rx, input_sample_rate) {
            let _ = app_err.emit("audio-error", serde_json::json!({ "message": e }));
        }
    })
}

fn stt_loop(
    app: AppHandle,
    ctx: WhisperContext,
    pcm_rx: Receiver<Vec<f32>>,
    input_sample_rate: u32,
) -> Result<(), String> {
    let mut state = ctx
        .create_state()
        .map_err(|e| format!("failed to create whisper state: {e}"))?;

    let mut buffer_16k: Vec<f32> = Vec::new();
    let mut last_decode = Instant::now() - INFER_EVERY;
    let mut last_text = String::new();

    while let Ok(chunk) = pcm_rx.recv() {
        let chunk_16k = resample_mono_to_16k(&chunk, input_sample_rate);
        push_ring(&mut buffer_16k, &chunk_16k);

        if last_decode.elapsed() < INFER_EVERY {
            continue;
        }
        last_decode = Instant::now();

        let text = match run_decode(&mut state, &buffer_16k) {
            Ok(t) => t,
            Err(e) => {
                let _ = app.emit(
                    "audio-error",
                    serde_json::json!({ "message": format!("whisper decode failed: {e}") }),
                );
                continue;
            }
        };

        if text != last_text {
            last_text = text.clone();
            let _ = app.emit(
                "transcript-update",
                TranscriptUpdate {
                    text,
                    is_final: false,
                },
            );
        }
    }

    // Channel closed: final pass (best effort).
    if buffer_16k.len() >= MIN_DECODE_SAMPLES {
        if let Ok(text) = run_decode(&mut state, &buffer_16k) {
            if !text.is_empty() {
                let _ = app.emit(
                    "transcript-update",
                    TranscriptUpdate {
                        text,
                        is_final: true,
                    },
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{resample_mono_to_16k, TranscriptUpdate};

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
    fn transcript_update_json_shape() {
        let u = TranscriptUpdate {
            text: "hello".into(),
            is_final: true,
        };
        let j = serde_json::to_value(&u).expect("serialize");
        assert_eq!(j["text"], "hello");
        assert_eq!(j["is_final"], true);
    }
}

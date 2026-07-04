//! Whisper inference thread: PCM → 16 kHz → rolling buffer → `transcript-update` (Task 4b).

use crate::audio::transcript_event::{
    classify_partial, audio_has_speech, normalize_transcript_candidate, TranscriptPartialKind,
};
use log::debug;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

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

static WHISPER_LOAD_GATE: Mutex<()> = Mutex::new(());

fn whisper_gpu_compile_supported() -> bool {
    cfg!(any(
        feature = "whisper-metal",
        feature = "whisper-cuda",
        feature = "whisper-vulkan"
    ))
}

/// Flash attention during CUDA init is release-only to reduce debug MSVC stack pressure.
#[cfg(feature = "whisper-cuda")]
pub fn whisper_flash_attn_enabled() -> bool {
    !cfg!(debug_assertions)
}

#[cfg(not(feature = "whisper-cuda"))]
pub fn whisper_flash_attn_enabled() -> bool {
    false
}

fn new_whisper_context(
    model_path: &str,
    params: WhisperContextParameters,
    use_gpu: bool,
) -> Result<WhisperContext, whisper_rs::WhisperError> {
    if use_gpu {
        crate::gpu_startup::with_ggml_cuda_init(true, || {
            WhisperContext::new_with_params(model_path, params)
        })
    } else {
        WhisperContext::new_with_params(model_path, params)
    }
}

/// Load Whisper weights (may take seconds on first GPU init). Call from a background thread.
/// Returns `(context, use_accelerator)` where `use_accelerator` is false after a GPU→CPU fallback.
///
/// Windows debug + `whisper-cuda`: intermittent `STATUS_STACK_BUFFER_OVERRUN` during preload —
/// mitigated by startup sequencing + debug flash-attn off; see
/// `jarvis/docs/bugs/BUG-debug-cuda-whisper-stack-buffer-overrun.md`.
pub fn load_whisper_context(
    model_path: &str,
    use_gpu: bool,
) -> Result<(WhisperContext, bool), String> {
    // CPU-only builds abort inside whisper.cpp when `use_gpu` is true (STATUS_BREAKPOINT on Windows).
    let use_gpu = use_gpu && whisper_gpu_compile_supported();
    if use_gpu {
        log::debug!("whisper: loading model with GPU backend");
    }
    let mut params = WhisperContextParameters::default();
    params.use_gpu(use_gpu);
    if use_gpu {
        params.gpu_device(0);
        if whisper_flash_attn_enabled() {
            params.flash_attn(true);
        }
    }
    match new_whisper_context(model_path, params, use_gpu) {
        Ok(ctx) => Ok((ctx, use_gpu)),
        Err(gpu_err) if use_gpu => {
            log::warn!("whisper gpu init failed; falling back to cpu: {gpu_err}");
            let mut cpu_params = WhisperContextParameters::default();
            cpu_params.use_gpu(false);
            new_whisper_context(model_path, cpu_params, false)
                .map(|ctx| (ctx, false))
                .map_err(|cpu_err| {
                    format!(
                        "failed to load whisper model (gpu error: {gpu_err}; cpu retry error: {cpu_err})"
                    )
                })
        }
        Err(e) => Err(format!("failed to load whisper model: {e}")),
    }
}

/// Serialize whisper.cpp init — concurrent `WhisperContext::new` calls can abort on Windows.
pub fn load_whisper_context_serialized(
    model_path: &str,
    use_gpu: bool,
) -> Result<(WhisperContext, bool), String> {
    let _gate = WHISPER_LOAD_GATE
        .lock()
        .map_err(|_| "whisper load mutex poisoned".to_string())?;
    load_whisper_context(model_path, use_gpu)
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

/// One-shot decode: fresh [`whisper_rs::WhisperState`] per call so sliding-window passes do not
/// reuse stale KV / prompt state (see `FullParams::set_no_context` / `set_single_segment`).
fn run_decode(
    ctx: &WhisperContext,
    audio_16k: &[f32],
    use_accelerator: bool,
) -> Result<String, whisper_rs::WhisperError> {
    if audio_16k.len() < MIN_DECODE_SAMPLES {
        return Ok(String::new());
    }
    if !audio_has_speech(audio_16k) {
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
    params.set_n_threads(whisper_decode_thread_count(use_accelerator));
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

pub fn spawn_stt_thread(
    app: AppHandle,
    ctx: WhisperContext,
    pcm_rx: Receiver<Vec<f32>>,
    input_sample_rate: u32,
    hud_session_id: u64,
    use_whisper_accelerator: bool,
    initial_audio_16k: Vec<f32>,
    for_dictation: bool,
    cancel: Arc<AtomicBool>,
) -> JoinHandle<()> {
    let app_err = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = stt_loop(
            app,
            ctx,
            pcm_rx,
            input_sample_rate,
            hud_session_id,
            use_whisper_accelerator,
            initial_audio_16k,
            for_dictation,
            cancel,
        ) {
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
    use_whisper_accelerator: bool,
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

        let text = match run_decode(&ctx, buffer_16k, use_whisper_accelerator) {
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
        if let Ok(text) = run_decode(&ctx, &buffer_16k, use_whisper_accelerator) {
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
    use std::path::PathBuf;

    use super::{
        normalize_peak_f32, parse_local_whisper_model_id, resample_mono_to_16k,
        SilenceResetTracker, TranscriptPartialKind, TranscriptUpdate, TARGET_RATE,
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
        let v: TranscriptUpdate = serde_json::from_str(
            r#"{"text":"hi","is_final":false,"hud_session_id":1}"#,
        )
        .expect("deserialize");
        assert_eq!(v.kind, TranscriptPartialKind::Growth);
    }

    /// Manual: `cargo test manual_load_whisper_tiny_model -- --ignored --nocapture`
    #[test]
    #[ignore = "loads bundled whisper weights (~75 MiB); run manually when debugging STT crashes"]
    fn manual_load_whisper_tiny_model() {
        manual_load_whisper_tiny_model_impl_on_loader_thread(false);
    }

    #[cfg(feature = "whisper-cuda")]
    /// Manual: `cargo test manual_load_whisper_tiny_model_gpu -- --ignored --nocapture`
    #[test]
    #[ignore = "loads bundled whisper weights on CUDA; run manually when debugging GPU STT crashes"]
    fn manual_load_whisper_tiny_model_gpu() {
        manual_load_whisper_tiny_model_impl_on_loader_thread(true);
    }

    #[cfg(feature = "llm-local")]
    /// Manual: `cargo test manual_load_whisper_after_llama_backend -- --ignored --nocapture`
    #[test]
    #[ignore = "probes ggml clash between llama-cpp and whisper in dev builds"]
    fn manual_load_whisper_after_llama_backend() {
        llama_cpp_2::llama_backend::LlamaBackend::init().expect("llama backend init");
        manual_load_whisper_tiny_model_impl_on_loader_thread(false);
    }

    fn manual_load_whisper_tiny_model_impl_on_loader_thread(use_gpu: bool) {
        let model = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(whisper_model_filename(super::DEFAULT_LOCAL_WHISPER_MODEL));
        assert!(
            model.is_file(),
            "missing {}; run scripts/download-model.ps1",
            model.display()
        );
        let model_path = model.to_string_lossy().to_string();
        let result = crate::gpu_startup::spawn_whisper_loader_thread("manual-whisper-load", move || {
            super::load_whisper_context_serialized(model_path.as_str(), use_gpu)
        })
        .expect("spawn loader thread")
        .join()
        .expect("join loader thread");
        match result {
            Ok(_) => eprintln!("whisper model load ok (use_gpu={use_gpu})"),
            Err(e) => panic!("whisper model load failed (use_gpu={use_gpu}): {e}"),
        }
    }

    #[test]
    fn whisper_flash_attn_enabled_only_in_release_cuda() {
        let enabled = super::whisper_flash_attn_enabled();
        #[cfg(feature = "whisper-cuda")]
        {
            assert_eq!(enabled, !cfg!(debug_assertions));
        }
        #[cfg(not(feature = "whisper-cuda"))]
        {
            assert!(!enabled);
        }
    }

    #[test]
    fn whisper_gpu_compile_supported_matches_feature_flags() {
        let expected = cfg!(any(
            feature = "whisper-metal",
            feature = "whisper-cuda",
            feature = "whisper-vulkan"
        ));
        assert_eq!(super::whisper_gpu_compile_supported(), expected);
    }

    #[test]
    fn manifest_dir_resources_points_at_bundled_filename() {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(whisper_model_filename(super::DEFAULT_LOCAL_WHISPER_MODEL));
        assert!(p.ends_with(whisper_model_filename(super::DEFAULT_LOCAL_WHISPER_MODEL).as_str()));
    }

    #[test]
    fn normalize_peak_does_not_boost_sub_speech_floor() {
        let quiet = vec![0.005f32; 100];
        let n = normalize_peak_f32(&quiet);
        assert_eq!(n, quiet);
    }

    #[test]
    fn silence_tracker_resets_after_long_silence() {
        let mut t = SilenceResetTracker::for_mode(false);
        let silence = vec![0.0f32; (TARGET_RATE as usize) / 2];
        assert!(!t.push_and_should_reset(&silence));
        assert!(t.push_and_should_reset(&silence));
    }

    #[test]
    fn silence_tracker_dictation_waits_longer() {
        let mut cmd = SilenceResetTracker::for_mode(false);
        let mut dict = SilenceResetTracker::for_mode(true);
        let silence = vec![0.0f32; (TARGET_RATE as usize) / 2];
        assert!(!cmd.push_and_should_reset(&silence));
        assert!(!dict.push_and_should_reset(&silence));
        assert!(cmd.push_and_should_reset(&silence));
        assert!(!dict.push_and_should_reset(&silence));
        assert!(!dict.push_and_should_reset(&silence));
        assert!(dict.push_and_should_reset(&silence));
    }

    #[test]
    fn silence_tracker_clears_after_loud_chunk() {
        let mut t = SilenceResetTracker::for_mode(false);
        let silence = vec![0.0f32; (TARGET_RATE as usize) / 2];
        let loud = vec![0.5f32; (TARGET_RATE as usize) / 20];
        assert!(!t.push_and_should_reset(&silence));
        assert!(!t.push_and_should_reset(&loud));
        assert!(!t.push_and_should_reset(&silence));
        assert!(t.push_and_should_reset(&silence));
    }
}

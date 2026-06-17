//! Whisper inference thread: PCM → 16 kHz → rolling buffer → `transcript-update` (Task 4b).

use log::debug;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub const DEFAULT_LOCAL_WHISPER_MODEL: &str = "tiny.en";
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

pub fn whisper_model_filename(model_id: &str) -> String {
    format!("ggml-{}.bin", parse_local_whisper_model_id(Some(model_id)))
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
/// Clear stale decode context after this much continuous silence.
const SILENCE_RESET_AFTER: Duration = Duration::from_millis(900);

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
    pub text: String,
    pub is_final: bool,
    /// Must match the HUD `session_id` when the mic pipeline for this listen was started.
    /// Ignores emissions from detached STT threads after pipeline teardown (`mem::forget` join handle).
    pub hud_session_id: u64,
}

/// Tauri `resource_dir()` may not match `src-tauri/resources/` during `tauri dev`; keep a crate-relative fallback.
fn whisper_model_candidates(app: &AppHandle, model_id: &str) -> Vec<PathBuf> {
    let filename = whisper_model_filename(model_id);
    let mut out = Vec::new();
    if let Ok(dir) = app.path().resource_dir() {
        out.push(dir.join(&filename));
    }
    out.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(filename),
    );
    out
}

fn local_whisper_model_id_from_app(app: &AppHandle) -> String {
    crate::open_db_connection(app)
        .ok()
        .and_then(|conn| crate::db::get_app_settings(&conn).ok())
        .map(|s| s.local_whisper_model)
        .unwrap_or_else(|| DEFAULT_LOCAL_WHISPER_MODEL.to_string())
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

pub fn resolve_whisper_model_path_for_id(
    app: &AppHandle,
    model_id: &str,
) -> Result<PathBuf, String> {
    let model_id = parse_local_whisper_model_id(Some(model_id));
    let filename = whisper_model_filename(model_id);
    let candidates = whisper_model_candidates(app, model_id);
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
        "Whisper model `{filename}` not found (tried: {tried}). From the jarvis folder run `npm run fetch-whisper-models` or `.\\scripts\\download-model.ps1 -Model {model_id}`.",
    ))
}

pub fn resolve_whisper_model_path(app: &AppHandle) -> Result<PathBuf, String> {
    let model_id = local_whisper_model_id_from_app(app);
    resolve_whisper_model_path_for_id(app, model_id.as_str())
}

pub fn is_whisper_model_available(app: &AppHandle, model_id: &str) -> bool {
    resolve_whisper_model_path_for_id(app, model_id).is_ok()
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
fn run_decode(
    ctx: &WhisperContext,
    audio_16k: &[f32],
    use_accelerator: bool,
) -> Result<String, whisper_rs::WhisperError> {
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
pub(crate) struct SilenceResetTracker {
    silent_samples: usize,
}

impl SilenceResetTracker {
    fn new() -> Self {
        Self::default()
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
    use_whisper_accelerator: bool,
    initial_audio_16k: Vec<f32>,
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
) -> Result<(), String> {
    let had_preroll = !initial_audio_16k.is_empty();
    let mut buffer_16k: Vec<f32> = initial_audio_16k;
    let mut last_decode = Instant::now();
    let mut last_text = String::new();
    let mut silence_reset = SilenceResetTracker::new();
    let mut first_decode_pending = true;
    if had_preroll {
        last_decode = Instant::now() - FIRST_INFER_AFTER;
    }

    let maybe_decode = |buffer_16k: &mut Vec<f32>,
                            last_decode: &mut Instant,
                            last_text: &mut String,
                            first_decode_pending: &mut bool|
     -> Result<(), String> {
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
            if text == *last_text {
                return Ok(());
            }
            *last_text = text.clone();
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
        Ok(())
    };

    if !buffer_16k.is_empty() {
        maybe_decode(
            &mut buffer_16k,
            &mut last_decode,
            &mut last_text,
            &mut first_decode_pending,
        )?;
    }

    while let Ok(chunk) = pcm_rx.recv() {
        let chunk_16k = resample_mono_to_16k(&chunk, input_sample_rate);
        if silence_reset.push_and_should_reset(&chunk_16k) {
            debug!("stt: silence gap reached; reset rolling decode context");
            buffer_16k.clear();
            last_text.clear();
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

    // Channel closed: final pass (best effort).
    if buffer_16k.len() >= MIN_DECODE_SAMPLES {
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
        normalize_peak_f32, normalize_transcript_candidate, parse_local_whisper_model_id,
        resample_mono_to_16k, whisper_model_filename, SilenceResetTracker, TranscriptUpdate,
        TARGET_RATE,
    };

    #[test]
    fn parse_local_whisper_model_id_normalizes() {
        assert_eq!(parse_local_whisper_model_id(None), "tiny.en");
        assert_eq!(parse_local_whisper_model_id(Some("base.en")), "base.en");
        assert_eq!(parse_local_whisper_model_id(Some("bogus")), "tiny.en");
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
            .join(super::whisper_model_filename(super::DEFAULT_LOCAL_WHISPER_MODEL));
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
            .join(super::whisper_model_filename(super::DEFAULT_LOCAL_WHISPER_MODEL));
        assert!(p.ends_with(super::whisper_model_filename(super::DEFAULT_LOCAL_WHISPER_MODEL).as_str()));
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

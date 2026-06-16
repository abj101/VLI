//! Serialize ggml-CUDA init across whisper-rs and llama-cpp, and coordinate launch preload order.

use crate::db::AppSettings;
use log::warn;
use std::sync::Mutex;

/// whisper.cpp init uses deep native stacks on Windows MSVC (see BUG-debug-cuda-whisper-stack-buffer-overrun).
pub const WHISPER_LOADER_STACK: usize = 32 * 1024 * 1024;

static GGML_CUDA_INIT_GATE: Mutex<()> = Mutex::new(());

/// `true` when this binary links at least one ggml-CUDA backend crate.
pub fn ggml_cuda_coordination_enabled() -> bool {
    cfg!(any(feature = "whisper-cuda", feature = "llm-cuda"))
}

/// Serialize CUDA init across independent whisper.cpp and llama.cpp ggml builds.
pub fn with_ggml_cuda_init<R>(use_gpu: bool, f: impl FnOnce() -> R) -> R {
    if use_gpu && ggml_cuda_coordination_enabled() {
        let _guard = GGML_CUDA_INIT_GATE
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        f()
    } else {
        f()
    }
}

pub fn whisper_gpu_compile_supported() -> bool {
    cfg!(any(
        feature = "whisper-metal",
        feature = "whisper-cuda",
        feature = "whisper-vulkan"
    ))
}

pub fn should_run_whisper_preload_at_launch(settings: &AppSettings, cache_warm: bool) -> bool {
    settings.stt_provider == "local" && !cache_warm
}

pub fn whisper_preload_use_gpu(settings: &AppSettings) -> bool {
    settings.local_whisper_use_gpu && whisper_gpu_compile_supported()
}

/// Defer wake until Whisper GPU preload completes (reduces concurrent native load during init).
pub fn defer_wake_for_gpu_whisper(settings: &AppSettings, cache_warm: bool) -> bool {
    should_run_whisper_preload_at_launch(settings, cache_warm)
        && whisper_preload_use_gpu(settings)
}

pub fn should_router_warmup_at_launch(settings: &AppSettings) -> bool {
    settings.llm_router_tier2_enabled && settings.llm_router_warmup_on_launch
}

/// Spawn a named thread with [`WHISPER_LOADER_STACK`] for whisper.cpp / ggml native init.
pub fn spawn_whisper_loader_thread<F, T>(
    name: impl Into<String>,
    f: F,
) -> std::io::Result<std::thread::JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .name(name.into())
        .stack_size(WHISPER_LOADER_STACK)
        .spawn(f)
}

/// Run coordinated background work (whisper preload → router warmup → optional wake).
pub fn spawn_startup_sequence(
    needs_background_thread: bool,
    use_whisper_stack: bool,
    work: impl FnOnce() + Send + 'static,
) {
    if !needs_background_thread {
        return;
    }

    let builder = std::thread::Builder::new().name("jarvis-gpu-startup".into());
    let spawn_result = if use_whisper_stack {
        builder.stack_size(WHISPER_LOADER_STACK).spawn(work)
    } else {
        builder.spawn(work)
    };
    if spawn_result.is_err() {
        warn!("gpu-startup: failed to spawn coordinator thread");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::settings::DEFAULT_LLM_ROUTER_CONFIDENCE_THRESHOLD;

    fn base_settings() -> AppSettings {
        AppSettings {
            wake_engine: "oww".into(),
            oww_threshold: 0.7,
            stt_provider: "local".into(),
            remote_stt_url: String::new(),
            remote_stt_model: None,
            remote_stt_timeout_secs: 30,
            remote_stt_key_stored: false,
            local_whisper_use_gpu: false,
            llm_router_tier2_enabled: false,
            llm_router_warmup_on_launch: false,
            llm_router_confidence_threshold: DEFAULT_LLM_ROUTER_CONFIDENCE_THRESHOLD,
            llm_router_model_path: None,
        }
    }

    #[test]
    fn whisper_loader_stack_is_large_enough_for_windows_cuda() {
        assert!(WHISPER_LOADER_STACK >= 32 * 1024 * 1024);
    }

    #[test]
    fn cuda_startup_coordination_matches_cuda_features() {
        let expected = cfg!(any(feature = "whisper-cuda", feature = "llm-cuda"));
        assert_eq!(ggml_cuda_coordination_enabled(), expected);
    }

    #[test]
    fn startup_sequence_defers_wake_when_gpu_whisper() {
        let mut s = base_settings();
        s.local_whisper_use_gpu = true;
        let defer = defer_wake_for_gpu_whisper(&s, false);
        if whisper_gpu_compile_supported() {
            assert!(defer);
        } else {
            assert!(!defer);
        }
    }

    #[test]
    fn startup_sequence_does_not_defer_wake_for_cpu_whisper() {
        let s = base_settings();
        assert!(!defer_wake_for_gpu_whisper(&s, false));
    }

    #[test]
    fn startup_sequence_skips_whisper_preload_when_cache_warm() {
        let mut s = base_settings();
        s.local_whisper_use_gpu = true;
        assert!(!should_run_whisper_preload_at_launch(&s, true));
        assert!(!defer_wake_for_gpu_whisper(&s, true));
    }
}

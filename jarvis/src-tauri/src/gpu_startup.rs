//! Serialize the in-process llama.cpp CUDA init and coordinate its launch warmup.

use crate::db::AppSettings;
use log::warn;
use std::sync::Mutex;

/// llama.cpp init uses deep native stacks on Windows MSVC.
pub const LLM_LOADER_STACK: usize = 32 * 1024 * 1024;

static GGML_CUDA_INIT_GATE: Mutex<()> = Mutex::new(());

/// `true` when the binary links llama.cpp's CUDA backend.
pub fn ggml_cuda_coordination_enabled() -> bool {
    cfg!(feature = "llm-cuda")
}

/// Serialize CUDA init across llama.cpp ggml users.
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

pub fn should_router_warmup_at_launch(settings: &AppSettings) -> bool {
    settings.llm_router_tier2_enabled && settings.llm_router_warmup_on_launch
}

/// Spawn the startup coordinator on a larger stack when it will initialize llama.cpp.
pub fn spawn_startup_sequence(
    needs_background_thread: bool,
    use_llm_stack: bool,
    work: impl FnOnce() + Send + 'static,
) {
    if !needs_background_thread {
        return;
    }
    let builder = std::thread::Builder::new().name("jarvis-gpu-startup".into());
    let spawn_result = if use_llm_stack {
        builder.stack_size(LLM_LOADER_STACK).spawn(work)
    } else {
        builder.spawn(work)
    };
    if spawn_result.is_err() {
        warn!("gpu-startup: failed to spawn coordinator thread");
    }
}

//! Stubs when `llm-local` is not enabled.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tauri::AppHandle;

#[allow(dead_code)]
pub const ROUTER_MODEL_FILE: &str = "qwen2.5-0.5b-instruct-q4_k_m.gguf";
#[allow(dead_code)]
pub const COMPOSER_MODEL_FILE: &str = "qwen2.5-3b-instruct-q4_k_m.gguf";
pub const COMPOSER_DEFAULT_MAX_TOKENS: i32 = 512;
pub const COMPOSER_MAX_MAX_TOKENS: i32 = 768;
pub const COMPOSER_INFER_TIMEOUT: Duration = Duration::from_secs(90);

pub struct ComposerInferWorker;

impl ComposerInferWorker {
    pub fn spawn(_model_path: String, _use_gpu: bool) -> Result<Self, String> {
        Err(
            "LLM composer is not compiled into this build (missing `llm-local` feature).".into(),
        )
    }

    pub fn infer(
        &self,
        _prompt: &str,
        _max_tokens: i32,
        _cancel: Arc<AtomicBool>,
    ) -> Result<String, String> {
        Err(
            "LLM composer is not compiled into this build (missing `llm-local` feature).".into(),
        )
    }
}

pub fn llm_compile_backend() -> &'static str {
    "none"
}

pub fn llm_gpu_runtime_available(_backend: &str) -> bool {
    false
}

pub fn resolve_router_model_path(_app: &AppHandle, _custom_path: Option<&str>) -> Result<PathBuf, String> {
    Err(
        "LLM router is not compiled into this build (missing `llm-local` feature).".into(),
    )
}

pub fn resolve_composer_model_path(_app: &AppHandle, _custom_path: Option<&str>) -> Result<PathBuf, String> {
    Err(
        "LLM composer is not compiled into this build (missing `llm-local` feature).".into(),
    )
}

pub fn infer_router_completion(_model_path: &str, _prompt: &str, _use_gpu: bool) -> Result<String, String> {
    Err(
        "LLM router is not compiled into this build (missing `llm-local` feature).".into(),
    )
}

pub fn infer_composer_completion(
    _worker: &ComposerInferWorker,
    _prompt: &str,
    _max_tokens: i32,
    _cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    Err(
        "LLM composer is not compiled into this build (missing `llm-local` feature).".into(),
    )
}

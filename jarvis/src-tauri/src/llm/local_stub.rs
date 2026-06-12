//! Stubs when `llm-local` is not enabled.

use std::path::PathBuf;
use tauri::AppHandle;

#[allow(dead_code)]
pub const ROUTER_MODEL_FILE: &str = "qwen2.5-0.5b-instruct-q4_k_m.gguf";

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

pub fn infer_router_completion(_model_path: &str, _prompt: &str, _use_gpu: bool) -> Result<String, String> {
    Err(
        "LLM router is not compiled into this build (missing `llm-local` feature).".into(),
    )
}

//! On-device LLM tool router (Phase C). Inference is behind `llm-local`.

pub mod prompt;
pub mod router;
pub mod tauri_cmds;

#[cfg(feature = "llm-local")]
pub mod local;

#[cfg(not(feature = "llm-local"))]
mod local_stub;

#[cfg(feature = "llm-local")]
pub use local::{
    infer_router_completion, llm_compile_backend, llm_gpu_runtime_available, resolve_router_model_path,
};

#[cfg(not(feature = "llm-local"))]
pub use local_stub::{
    infer_router_completion, llm_compile_backend, llm_gpu_runtime_available, resolve_router_model_path,
};

pub use router::{
    route_transcript_with_infer, RouterError, RouterErrorCode, RouterInfer, RouterRouteResult,
};
pub use tauri_cmds::RouterModelCache;

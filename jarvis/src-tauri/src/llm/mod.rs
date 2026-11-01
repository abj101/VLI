//! On-device LLM tool router.

pub mod composer;
#[cfg(test)]
pub mod composer_expect;
pub mod composer_prompt;
pub mod composer_validate;
pub mod decider;
pub mod decider_args;
pub mod focus_context;

#[cfg(test)]
mod composer_eval;
pub mod intent_validate;
pub mod prompt;
pub mod router;
#[cfg(test)]
mod router_eval;
pub mod tauri_cmds;

pub mod local;
pub use local::{
    ensure_router_server, infer_composer_completion, infer_router_completion,
    llm_gpu_runtime_available, resolve_composer_model_path, resolve_router_model_path,
    ComposerInferWorker, COMPOSER_DEFAULT_MAX_TOKENS, COMPOSER_INFER_TIMEOUT,
};

pub use composer::{
    generate_automation_with_infer, ComposerError, ComposerErrorCode, ComposerInfer,
    GenerateAutomationResult,
};
pub use intent_validate::normalize_router_tool_calls;
pub use router::{
    route_transcript_with_infer, RouterError, RouterErrorCode, RouterInfer, RouterRouteResult,
};
pub use tauri_cmds::{
    router_model_ready, spawn_router_preload, ComposerModelCache, RouterModelCache,
    ROUTER_LOADING_STATUS,
};

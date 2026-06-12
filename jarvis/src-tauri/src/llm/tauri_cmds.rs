//! Tauri IPC for the on-device LLM router.

use crate::db::{
    get_all_tools, get_app_settings,
    settings::DEFAULT_LLM_ROUTER_CONFIDENCE_THRESHOLD,
};
use crate::llm::{
    infer_router_completion, llm_compile_backend, llm_gpu_runtime_available,
    resolve_router_model_path, route_transcript_with_infer, RouterError, RouterErrorCode,
    RouterInfer, RouterRouteResult,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

/// Warm-loaded router weights reused after warmup.
pub struct RouterModelCache {
    pub path: Mutex<Option<String>>,
    pub warmed: AtomicBool,
}

impl RouterModelCache {
    pub fn new() -> Self {
        Self {
            path: Mutex::new(None),
            warmed: AtomicBool::new(false),
        }
    }

    pub fn mark_warmed(&self, path: String) {
        if let Ok(mut slot) = self.path.lock() {
            *slot = Some(path);
        }
        self.warmed.store(true, Ordering::SeqCst);
    }

    pub fn is_warmed(&self) -> bool {
        self.warmed.load(Ordering::SeqCst)
    }
}

impl Default for RouterModelCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterStatus {
    pub feature_compiled: bool,
    pub compile_backend: String,
    pub runtime_available: bool,
    pub model_present: bool,
    pub model_path: Option<String>,
    pub tier2_enabled: bool,
    pub confidence_threshold: f32,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterWarmupPayload {
    pub ready: bool,
    pub message: String,
}

struct LocalInfer {
    model_path: String,
    use_gpu: bool,
}

impl RouterInfer for LocalInfer {
    fn infer(&self, prompt: &str) -> Result<String, String> {
        infer_router_completion(&self.model_path, prompt, self.use_gpu)
    }
}

fn open_db(app: &AppHandle) -> Result<rusqlite::Connection, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    rusqlite::Connection::open(dir.join("jarvis.db")).map_err(|e| e.to_string())
}

fn llm_backend_label(backend: &str) -> &'static str {
    match backend {
        "metal" => "Metal",
        "cuda" => "CUDA",
        "vulkan" => "Vulkan",
        _ => "CPU",
    }
}

fn router_status_inner(app: &AppHandle) -> RouterStatus {
    let feature_compiled = cfg!(feature = "llm-local");
    let compile_backend = if feature_compiled {
        llm_compile_backend().to_string()
    } else {
        "none".to_string()
    };
    let runtime_available =
        feature_compiled && llm_gpu_runtime_available(llm_compile_backend());

    let (tier2_enabled, confidence_threshold, custom_model_path) =
        match open_db(app).and_then(|conn| {
            let settings = get_app_settings(&conn).map_err(|e| e.to_string())?;
            Ok((
                settings.llm_router_tier2_enabled,
                settings.llm_router_confidence_threshold,
                settings.llm_router_model_path,
            ))
        }) {
            Ok(v) => v,
            Err(_) => (false, DEFAULT_LLM_ROUTER_CONFIDENCE_THRESHOLD, None),
        };

    let model_path = if feature_compiled {
        resolve_router_model_path(app, custom_model_path.as_deref()).ok()
    } else {
        None
    };
    let model_present = model_path.is_some();

    let message = if !feature_compiled {
        Some("This build was compiled without the `llm-local` feature.".into())
    } else if !model_present {
        Some("Router model not found. Run scripts/download-router-model.ps1.".into())
    } else if compile_backend != "none" && runtime_available {
        Some(format!(
            "Router GPU backend ready: {}.",
            llm_backend_label(&compile_backend)
        ))
    } else if compile_backend != "none" {
        Some(format!(
            "{} backend is compiled, but runtime was not detected; router will use CPU.",
            llm_backend_label(&compile_backend)
        ))
    } else {
        Some("Router will run on CPU.".into())
    };

    RouterStatus {
        feature_compiled,
        compile_backend,
        runtime_available,
        model_present,
        model_path: model_path.map(|p| p.display().to_string()),
        tier2_enabled,
        confidence_threshold,
        message,
    }
}

#[tauri::command]
pub fn router_status(app: AppHandle) -> RouterStatus {
    router_status_inner(&app)
}

fn warmup_router_blocking(app: AppHandle) -> RouterWarmupPayload {
    let status = router_status_inner(&app);
    if !status.feature_compiled {
        return RouterWarmupPayload {
            ready: false,
            message: "This build has no LLM router feature.".into(),
        };
    }
    let model_path = match status.model_path {
        Some(p) => p,
        None => {
            return RouterWarmupPayload {
                ready: false,
                message: "Router model file is missing.".into(),
            };
        }
    };
    let use_gpu = status.runtime_available;
    match infer_router_completion(&model_path, "warmup", use_gpu) {
        Ok(_) => {
            if let Some(cache) = app.try_state::<RouterModelCache>() {
                cache.mark_warmed(model_path);
            }
            RouterWarmupPayload {
                ready: true,
                message: format!("Router model ready ({}).", llm_backend_label(&status.compile_backend)),
            }
        }
        Err(msg) => RouterWarmupPayload {
            ready: false,
            message: format!("Router warmup failed: {msg}"),
        },
    }
}

/// Background warmup when Tier 2 and warmup-on-launch are enabled (app startup).
pub fn spawn_router_warmup_on_launch(app: &AppHandle, settings: &crate::db::AppSettings) {
    if !settings.llm_router_tier2_enabled || !settings.llm_router_warmup_on_launch {
        return;
    }
    let status = router_status_inner(app);
    if !status.feature_compiled || !status.model_present {
        return;
    }
    if let Some(cache) = app.try_state::<RouterModelCache>() {
        if cache.is_warmed() {
            return;
        }
    }
    let app_bg = app.clone();
    std::thread::spawn(move || {
        let payload = warmup_router_blocking(app_bg.clone());
        let _ = app_bg.emit("router-warmup", &payload);
    });
}

#[tauri::command]
pub fn router_warmup(app: AppHandle, cache: State<'_, RouterModelCache>) -> RouterWarmupPayload {
    let status = router_status_inner(&app);
    if !status.feature_compiled {
        return RouterWarmupPayload {
            ready: false,
            message: "This build has no LLM router feature.".into(),
        };
    }
    if !status.model_present {
        return RouterWarmupPayload {
            ready: false,
            message: "Router model file is missing.".into(),
        };
    }
    if cache.is_warmed() {
        return RouterWarmupPayload {
            ready: true,
            message: "Router model already warm.".into(),
        };
    }
    let app_bg = app.clone();
    std::thread::spawn(move || {
        let payload = warmup_router_blocking(app_bg.clone());
        let _ = app_bg.emit("router-warmup", &payload);
    });
    RouterWarmupPayload {
        ready: false,
        message: "Loading router model in background…".into(),
    }
}

#[tauri::command]
pub fn route_transcript(app: AppHandle, transcript: String) -> Result<RouterRouteResult, String> {
    if !cfg!(feature = "llm-local") {
        return Err(
            RouterError {
                code: RouterErrorCode::FeatureDisabled,
                message: "LLM router is not compiled into this build.".into(),
            }
            .into_string(),
        );
    }

    let conn = open_db(&app)?;
    let settings = get_app_settings(&conn).map_err(|e| e.to_string())?;
    if !settings.llm_router_tier2_enabled {
        return Err(
            RouterError {
                code: RouterErrorCode::Tier2Disabled,
                message: "Tier 2 LLM routing is disabled in settings.".into(),
            }
            .into_string(),
        );
    }

    let model_path = resolve_router_model_path(&app, settings.llm_router_model_path.as_deref())
        .map_err(|message| {
            RouterError {
                code: RouterErrorCode::ModelMissing,
                message,
            }
            .into_string()
        })?;

    let tools = get_all_tools(&conn).map_err(|e| e.to_string())?;
    let use_gpu = llm_gpu_runtime_available(llm_compile_backend());
    let infer = LocalInfer {
        model_path: model_path.display().to_string(),
        use_gpu,
    };
    route_transcript_with_infer(
        &conn,
        &tools,
        transcript.trim(),
        settings.llm_router_confidence_threshold,
        &infer,
    )
    .map_err(|e| e.into_string())
}

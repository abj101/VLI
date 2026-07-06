//! Tauri IPC for the on-device LLM router and command composer.

use crate::db::{
    get_all_tools, get_app_settings,
    settings::DEFAULT_LLM_ROUTER_CONFIDENCE_THRESHOLD,
};
use crate::llm::{
    generate_automation_with_infer, infer_composer_completion, infer_router_completion,
    llm_compile_backend, llm_gpu_runtime_available, resolve_composer_model_path,
    resolve_router_model_path, route_transcript_with_infer, ComposerError, ComposerErrorCode,
    ComposerInfer, ComposerInferWorker, GenerateAutomationResult, RouterError, RouterErrorCode,
    RouterInfer, RouterRouteResult, COMPOSER_DEFAULT_MAX_TOKENS, COMPOSER_INFER_TIMEOUT,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

pub const ROUTER_LOADING_STATUS: &str = "Router model loading…";
pub const COMPOSER_LOADING_STATUS: &str = "Composer model loading…";

/// Tracks background router warmup so Tier 2 can gate until the first load completes.
pub struct RouterModelCache {
    pub path: Mutex<Option<String>>,
    warmed: AtomicBool,
    loading: AtomicBool,
}

impl RouterModelCache {
    pub fn new() -> Self {
        Self {
            path: Mutex::new(None),
            warmed: AtomicBool::new(false),
            loading: AtomicBool::new(false),
        }
    }

    pub fn is_warmed(&self) -> bool {
        self.warmed.load(Ordering::SeqCst)
    }

    pub fn is_loading(&self) -> bool {
        self.loading.load(Ordering::SeqCst)
    }

    /// Returns true when this caller should start the background load.
    pub fn try_begin_loading(&self) -> bool {
        if self.is_warmed() {
            return false;
        }
        self.loading
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    fn finish_loading(&self, warmed: bool, path: Option<String>) {
        if warmed {
            if let Some(p) = path {
                if let Ok(mut slot) = self.path.lock() {
                    *slot = Some(p);
                }
            }
            self.warmed.store(true, Ordering::SeqCst);
        }
        self.loading.store(false, Ordering::SeqCst);
    }

    pub fn mark_warmed(&self, path: String) {
        self.finish_loading(true, Some(path));
    }
}

pub fn emit_router_loading_notice(app: &AppHandle) {
    let _ = app.emit(
        "action-status",
        serde_json::json!({ "text": ROUTER_LOADING_STATUS }),
    );
}

pub fn clear_router_loading_notice(app: &AppHandle) {
    let _ = app.emit("action-status", serde_json::json!({ "text": "" }));
}

fn finish_router_warmup_emit(app: &AppHandle, payload: &RouterWarmupPayload) {
    if payload.ready {
        clear_router_loading_notice(app);
    } else {
        if let Some(cache) = app.try_state::<RouterModelCache>() {
            cache.finish_loading(false, None);
        }
        let _ = app.emit(
            "action-error",
            serde_json::json!({ "message": payload.message }),
        );
    }
    let _ = app.emit("router-warmup", payload);
}

/// Start background router warmup when Tier 2 is enabled and the GGUF is present.
pub fn spawn_router_preload(app: &AppHandle) -> bool {
    let status = router_status_inner(app);
    if !status.feature_compiled || !status.model_present {
        return false;
    }
    let Some(cache) = app.try_state::<RouterModelCache>() else {
        return false;
    };
    if !cache.try_begin_loading() {
        return false;
    }
    emit_router_loading_notice(app);
    let app_bg = app.clone();
    std::thread::spawn(move || {
        let payload = warmup_router_blocking(app_bg.clone());
        finish_router_warmup_emit(&app_bg, &payload);
    });
    true
}

/// Blocking router warmup for the launch coordinator (caller is already on a background thread).
pub(crate) fn run_router_warmup_at_launch_blocking(app: &AppHandle) {
    let status = router_status_inner(app);
    if !status.feature_compiled || !status.model_present {
        return;
    }
    let Some(cache) = app.try_state::<RouterModelCache>() else {
        return;
    };
    if !cache.try_begin_loading() {
        return;
    }
    emit_router_loading_notice(app);
    let payload = warmup_router_blocking(app.clone());
    finish_router_warmup_emit(app, &payload);
}

/// Tier 2 routing is allowed only after the first router warmup completed.
pub fn router_model_ready(app: &AppHandle) -> bool {
    app.try_state::<RouterModelCache>()
        .is_some_and(|cache| cache.is_warmed())
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
        Some("Router model not found. Run npm run fetch-models.".into())
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

pub(crate) fn warmup_router_blocking(app: AppHandle) -> RouterWarmupPayload {
    let status = router_status_inner(&app);
    if !status.feature_compiled {
        if let Some(cache) = app.try_state::<RouterModelCache>() {
            cache.finish_loading(false, None);
        }
        return RouterWarmupPayload {
            ready: false,
            message: "This build has no LLM router feature.".into(),
        };
    }
    let model_path = match status.model_path {
        Some(p) => p,
        None => {
            if let Some(cache) = app.try_state::<RouterModelCache>() {
                cache.finish_loading(false, None);
            }
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
                message: format!(
                    "Router model ready ({}).",
                    llm_backend_label(&status.compile_backend)
                ),
            }
        }
        Err(msg) => {
            if let Some(cache) = app.try_state::<RouterModelCache>() {
                cache.finish_loading(false, None);
            }
            RouterWarmupPayload {
                ready: false,
                message: format!("Router warmup failed: {msg}"),
            }
        }
    }
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
    if cache.is_loading() {
        return RouterWarmupPayload {
            ready: false,
            message: ROUTER_LOADING_STATUS.into(),
        };
    }
    if !spawn_router_preload(&app) {
        return RouterWarmupPayload {
            ready: false,
            message: "Router model file is missing.".into(),
        };
    }
    RouterWarmupPayload {
        ready: false,
        message: ROUTER_LOADING_STATUS.into(),
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
    if !router_model_ready(&app) {
        let _ = spawn_router_preload(&app);
        return Err(
            RouterError {
                code: RouterErrorCode::ModelLoading,
                message: ROUTER_LOADING_STATUS.into(),
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

// --- Command composer (editor NL → draft) ---

/// Tracks resident composer worker + warmup state.
pub struct ComposerModelCache {
    worker: Mutex<Option<ComposerInferWorker>>,
    path: Mutex<Option<String>>,
    warmed: AtomicBool,
    loading: AtomicBool,
    cancel_flag: Mutex<Arc<AtomicBool>>,
}

impl ComposerModelCache {
    pub fn new() -> Self {
        Self {
            worker: Mutex::new(None),
            path: Mutex::new(None),
            warmed: AtomicBool::new(false),
            loading: AtomicBool::new(false),
            cancel_flag: Mutex::new(Arc::new(AtomicBool::new(false))),
        }
    }

    pub fn is_warmed(&self) -> bool {
        self.warmed.load(Ordering::SeqCst)
    }

    pub fn is_loading(&self) -> bool {
        self.loading.load(Ordering::SeqCst)
    }

    pub fn try_begin_loading(&self) -> bool {
        if self.is_warmed() {
            return false;
        }
        self.loading
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    fn finish_loading(&self, warmed: bool, path: Option<String>, worker: Option<ComposerInferWorker>) {
        if warmed {
            if let Some(p) = path {
                if let Ok(mut slot) = self.path.lock() {
                    *slot = Some(p);
                }
            }
            if let (Some(w), Ok(mut slot)) = (worker, self.worker.lock()) {
                *slot = Some(w);
            }
            self.warmed.store(true, Ordering::SeqCst);
        }
        self.loading.store(false, Ordering::SeqCst);
    }

    fn invalidate_worker(&self) {
        if let Ok(mut slot) = self.worker.lock() {
            *slot = None;
        }
        self.warmed.store(false, Ordering::SeqCst);
        self.loading.store(false, Ordering::SeqCst);
        if let Ok(mut slot) = self.path.lock() {
            *slot = None;
        }
    }

    fn worker_for_path(&self, model_path: &str, use_gpu: bool) -> Result<(), String> {
        let needs_reload = self
            .path
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .is_none_or(|p| p != model_path);
        if needs_reload {
            self.invalidate_worker();
            let worker = ComposerInferWorker::spawn(model_path.to_string(), use_gpu)?;
            if let Ok(mut slot) = self.worker.lock() {
                *slot = Some(worker);
            }
            if let Ok(mut slot) = self.path.lock() {
                *slot = Some(model_path.to_string());
            }
            self.warmed.store(true, Ordering::SeqCst);
        } else if !self.is_warmed() {
            let worker = ComposerInferWorker::spawn(model_path.to_string(), use_gpu)?;
            if let Ok(mut slot) = self.worker.lock() {
                *slot = Some(worker);
            }
            if let Ok(mut slot) = self.path.lock() {
                *slot = Some(model_path.to_string());
            }
            self.warmed.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn with_worker<R>(&self, f: impl FnOnce(&ComposerInferWorker) -> R) -> Result<R, String> {
        let guard = self
            .worker
            .lock()
            .map_err(|_| "composer worker mutex poisoned".to_string())?;
        let worker = guard
            .as_ref()
            .ok_or_else(|| "composer model is not loaded".to_string())?;
        Ok(f(worker))
    }

    pub fn fresh_cancel_flag(&self) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut slot) = self.cancel_flag.lock() {
            *slot = Arc::clone(&flag);
        }
        flag
    }

    pub fn signal_cancel(&self) {
        if let Ok(slot) = self.cancel_flag.lock() {
            slot.store(true, Ordering::SeqCst);
        }
    }
}

impl Default for ComposerModelCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposerStatus {
    pub feature_compiled: bool,
    pub compile_backend: String,
    pub runtime_available: bool,
    pub model_present: bool,
    pub model_path: Option<String>,
    pub composer_enabled: bool,
    pub ready: bool,
    pub loading: bool,
    pub message: Option<String>,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposerWarmupPayload {
    pub ready: bool,
    pub message: String,
}

struct WorkerInfer<'a> {
    worker: &'a ComposerInferWorker,
    cancel: Arc<AtomicBool>,
}

impl ComposerInfer for WorkerInfer<'_> {
    fn infer(&self, prompt: &str) -> Result<String, String> {
        infer_composer_completion(
            self.worker,
            prompt,
            COMPOSER_DEFAULT_MAX_TOKENS,
            Arc::clone(&self.cancel),
        )
    }
}

fn composer_status_inner(app: &AppHandle) -> ComposerStatus {
    let feature_compiled = cfg!(feature = "llm-local");
    let compile_backend = if feature_compiled {
        llm_compile_backend().to_string()
    } else {
        "none".to_string()
    };
    let runtime_available =
        feature_compiled && llm_gpu_runtime_available(llm_compile_backend());

    let (composer_enabled, custom_model_path) = match open_db(app).and_then(|conn| {
        let settings = get_app_settings(&conn).map_err(|e| e.to_string())?;
        Ok((settings.llm_composer_enabled, settings.llm_composer_model_path))
    }) {
        Ok(v) => v,
        Err(_) => (false, None),
    };

    let model_path = if feature_compiled {
        resolve_composer_model_path(app, custom_model_path.as_deref()).ok()
    } else {
        None
    };
    let model_present = model_path.is_some();
    let (ready, loading) = app
        .try_state::<ComposerModelCache>()
        .map(|c| (c.is_warmed(), c.is_loading()))
        .unwrap_or((false, false));

    let message = if !feature_compiled {
        Some("This build was compiled without the `llm-local` feature.".into())
    } else if !composer_enabled {
        Some("Command composer is disabled in settings.".into())
    } else if !model_present {
        Some("Composer model not found. Run npm run fetch-models.".into())
    } else if loading {
        Some(COMPOSER_LOADING_STATUS.into())
    } else if ready {
        Some("Composer model ready.".into())
    } else if compile_backend != "none" && runtime_available {
        Some(format!(
            "Composer GPU backend ready: {}.",
            llm_backend_label(&compile_backend)
        ))
    } else {
        Some("Composer will run on CPU (slow for 3B — GPU recommended).".into())
    };

    ComposerStatus {
        feature_compiled,
        compile_backend,
        runtime_available,
        model_present,
        model_path: model_path.map(|p| p.display().to_string()),
        composer_enabled,
        ready,
        loading,
        message,
        platform: std::env::consts::OS.to_string(),
    }
}

fn finish_composer_warmup_emit(app: &AppHandle, payload: &ComposerWarmupPayload) {
  if payload.ready {
    let _ = app.emit("composer-warmup", payload);
  } else {
    if let Some(cache) = app.try_state::<ComposerModelCache>() {
      cache.finish_loading(false, None, None);
    }
    let _ = app.emit(
      "action-error",
      serde_json::json!({ "message": payload.message }),
    );
    let _ = app.emit("composer-warmup", payload);
  }
}

pub(crate) fn warmup_composer_blocking(app: &AppHandle) -> ComposerWarmupPayload {
    let status = composer_status_inner(app);
    if !status.feature_compiled {
        if let Some(cache) = app.try_state::<ComposerModelCache>() {
            cache.finish_loading(false, None, None);
        }
        return ComposerWarmupPayload {
            ready: false,
            message: "This build has no LLM composer feature.".into(),
        };
    }
    if !status.composer_enabled {
        if let Some(cache) = app.try_state::<ComposerModelCache>() {
            cache.finish_loading(false, None, None);
        }
        return ComposerWarmupPayload {
            ready: false,
            message: "Command composer is disabled in settings.".into(),
        };
    }
    let model_path = match status.model_path {
        Some(p) => p,
        None => {
            if let Some(cache) = app.try_state::<ComposerModelCache>() {
                cache.finish_loading(false, None, None);
            }
            return ComposerWarmupPayload {
                ready: false,
                message: "Composer model file is missing.".into(),
            };
        }
    };
    let use_gpu = status.runtime_available;
    let Some(cache) = app.try_state::<ComposerModelCache>() else {
        return ComposerWarmupPayload {
            ready: false,
            message: "Composer cache unavailable.".into(),
        };
    };
    let cancel = cache.fresh_cancel_flag();
    match cache.worker_for_path(&model_path, use_gpu).and_then(|()| {
        cache.with_worker(|worker| {
            infer_composer_completion(worker, "warmup", 8, cancel)
        })
    }) {
        Ok(Ok(_)) => {
            cache.finish_loading(true, Some(model_path), None);
            ComposerWarmupPayload {
                ready: true,
                message: format!(
                    "Composer model ready ({}).",
                    llm_backend_label(&status.compile_backend)
                ),
            }
        }
        Ok(Err(msg)) | Err(msg) => {
            cache.finish_loading(false, None, None);
            ComposerWarmupPayload {
                ready: false,
                message: format!("Composer warmup failed: {msg}"),
            }
        }
    }
}

pub fn spawn_composer_preload(app: &AppHandle) -> bool {
    let status = composer_status_inner(app);
    if !status.feature_compiled || !status.model_present || !status.composer_enabled {
        return false;
    }
    let Some(cache) = app.try_state::<ComposerModelCache>() else {
        return false;
    };
    if !cache.try_begin_loading() {
        return false;
    }
    let app_bg = app.clone();
    std::thread::spawn(move || {
        let payload = warmup_composer_blocking(&app_bg);
        finish_composer_warmup_emit(&app_bg, &payload);
    });
    true
}

#[tauri::command]
pub fn composer_status(app: AppHandle) -> ComposerStatus {
    composer_status_inner(&app)
}

#[tauri::command]
pub fn composer_warmup(
    app: AppHandle,
    cache: State<'_, ComposerModelCache>,
) -> ComposerWarmupPayload {
    let status = composer_status_inner(&app);
    if !status.feature_compiled {
        return ComposerWarmupPayload {
            ready: false,
            message: "This build has no LLM composer feature.".into(),
        };
    }
    if !status.composer_enabled {
        return ComposerWarmupPayload {
            ready: false,
            message: "Command composer is disabled in settings.".into(),
        };
    }
    if !status.model_present {
        return ComposerWarmupPayload {
            ready: false,
            message: "Composer model file is missing.".into(),
        };
    }
    if cache.is_warmed() {
        return ComposerWarmupPayload {
            ready: true,
            message: "Composer model already warm.".into(),
        };
    }
    if cache.is_loading() {
        return ComposerWarmupPayload {
            ready: false,
            message: COMPOSER_LOADING_STATUS.into(),
        };
    }
    if !spawn_composer_preload(&app) {
        return ComposerWarmupPayload {
            ready: false,
            message: "Composer model file is missing.".into(),
        };
    }
    ComposerWarmupPayload {
        ready: false,
        message: COMPOSER_LOADING_STATUS.into(),
    }
}

#[tauri::command]
pub fn cancel_generate_automation(cache: State<'_, ComposerModelCache>) {
    cache.signal_cancel();
}

#[tauri::command]
pub fn generate_automation(
    app: AppHandle,
    description: String,
    trigger: Option<String>,
    cache: State<'_, ComposerModelCache>,
) -> Result<GenerateAutomationResult, String> {
    if !cfg!(feature = "llm-local") {
        return Err(
            ComposerError {
                code: ComposerErrorCode::FeatureDisabled,
                message: "LLM composer is not compiled into this build.".into(),
            }
            .into_string(),
        );
    }

    let trimmed = description.trim();
    if trimmed.is_empty() {
        return Err(
            ComposerError {
                code: ComposerErrorCode::SchemaInvalid,
                message: "description cannot be empty".into(),
            }
            .into_string(),
        );
    }

    let conn = open_db(&app)?;
    let settings = get_app_settings(&conn).map_err(|e| e.to_string())?;
    if !settings.llm_composer_enabled {
        return Err(
            ComposerError {
                code: ComposerErrorCode::FeatureDisabled,
                message: "Command composer is disabled in settings.".into(),
            }
            .into_string(),
        );
    }

    let model_path = resolve_composer_model_path(&app, settings.llm_composer_model_path.as_deref())
        .map_err(|message| {
            ComposerError {
                code: ComposerErrorCode::ModelMissing,
                message,
            }
            .into_string()
        })?;

    if cache.is_loading() && !cache.is_warmed() {
        return Err(
            ComposerError {
                code: ComposerErrorCode::ModelLoading,
                message: COMPOSER_LOADING_STATUS.into(),
            }
            .into_string(),
        );
    }

    let tools = get_all_tools(&conn).map_err(|e| e.to_string())?;
    let builtin_tool_names: Vec<String> = tools
        .iter()
        .filter(|tool| tool.builtin)
        .map(|tool| tool.name.clone())
        .collect();

    let use_gpu = llm_gpu_runtime_available(llm_compile_backend());
    let model_path_str = model_path.display().to_string();
    let cancel = cache.fresh_cancel_flag();
    let (tx, rx) = mpsc::channel();
    let description_owned = trimmed.to_string();
    let trigger_owned = trigger
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let app_bg = app.clone();

    std::thread::spawn(move || {
        let builtin_refs: Vec<&str> = builtin_tool_names.iter().map(String::as_str).collect();
        let trigger_ref = trigger_owned.as_deref();
        let result = (|| -> Result<GenerateAutomationResult, ComposerError> {
            let cache = app_bg
                .try_state::<ComposerModelCache>()
                .ok_or_else(|| ComposerError {
                    code: ComposerErrorCode::InferFailed,
                    message: "composer cache unavailable".into(),
                })?;
            cache.worker_for_path(&model_path_str, use_gpu).map_err(|message| {
                ComposerError {
                    code: ComposerErrorCode::InferFailed,
                    message,
                }
            })?;
            cache
                .with_worker(|worker| {
                    let infer = WorkerInfer {
                        worker,
                        cancel: Arc::clone(&cancel),
                    };
                    generate_automation_with_infer(
                        &description_owned,
                        trigger_ref,
                        &infer,
                        &builtin_refs,
                    )
                })
                .map_err(|message| ComposerError {
                    code: ComposerErrorCode::InferFailed,
                    message,
                })?
        })();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(COMPOSER_INFER_TIMEOUT) {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(err)) => Err(err.into_string()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            cache.signal_cancel();
            Err(
                ComposerError {
                    code: ComposerErrorCode::TimedOut,
                    message: "Composer inference timed out after 90 seconds.".into(),
                }
                .into_string(),
            )
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(
            ComposerError {
                code: ComposerErrorCode::InferFailed,
                message: "Composer inference thread ended unexpectedly.".into(),
            }
            .into_string(),
        ),
    }
}

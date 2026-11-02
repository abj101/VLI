//! Local GGUF inference through a persistent llama.cpp HTTP sidecar.
use crate::sidecar::{self, Sidecar};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Manager};

pub const DICTATION_MODEL_FILE: &str = "s1-mini-q4_k_m.gguf";
pub const ROUTER_MODEL_FILE: &str = "d1-2B-Q4_K_M.gguf";
pub const COMPOSER_MODEL_FILE: &str = "qwen2.5-3b-instruct-q4_k_m.gguf";
const ROUTER_N_CTX: u32 = 2048;
const COMPOSER_N_CTX: u32 = 4096;
const MAX_GENERATED_TOKENS: i32 = 256;
const ROUTER_SYSTEM_PROMPT: &str = "You route voice commands to JSON tool calls.";
pub const COMPOSER_DEFAULT_MAX_TOKENS: i32 = 512;
pub const COMPOSER_MAX_MAX_TOKENS: i32 = 768;
pub const COMPOSER_INFER_TIMEOUT: Duration = Duration::from_secs(90);
const SIDECAR_START_TIMEOUT: Duration = Duration::from_secs(180);
#[cfg(windows)]
const LLAMA_SERVER_EXE: &str = "llama/llama-server.exe";
#[cfg(not(windows))]
const LLAMA_SERVER_EXE: &str = "llama-server";

static DICTATION_SERVER: OnceLock<Mutex<Option<RouterServer>>> = OnceLock::new();
static ROUTER_SERVER: OnceLock<Mutex<Option<RouterServer>>> = OnceLock::new();
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

struct RouterServer {
    model_path: String,
    use_gpu: bool,
    sidecar: Sidecar,
}

pub fn set_app_handle(app: AppHandle) {
    let _ = APP_HANDLE.set(app);
}

pub fn llm_gpu_runtime_available(app: &AppHandle) -> bool {
    #[cfg(target_os = "windows")]
    {
        let cuda_dll = app
            .path()
            .resource_dir()
            .ok()
            .is_some_and(|dir| dir.join("bin/llama/cudart64_12.dll").is_file());
        let gpu = std::process::Command::new("nvidia-smi")
            .arg("-L")
            .output()
            .is_ok_and(|result| result.status.success());
        cuda_dll && gpu
    }
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        false
    }
}

fn model_candidates(
    app: &AppHandle,
    custom_path: Option<&str>,
    default_file: &str,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(path) = custom_path.map(str::trim).filter(|p| !p.is_empty()) {
        out.push(PathBuf::from(path));
    }
    if let Ok(dir) = app.path().resource_dir() {
        out.push(dir.join(default_file));
    }
    out.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(default_file),
    );
    out
}

fn resolve_model(
    app: &AppHandle,
    custom_path: Option<&str>,
    default_file: &str,
    label: &str,
) -> Result<PathBuf, String> {
    let candidates = model_candidates(app, custom_path, default_file);
    if let Some(path) = candidates.iter().find(|path| path.is_file()) {
        return Ok(path.clone());
    }
    let tried = candidates
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!("{label} model `{default_file}` not found (tried: {tried}). Run `npm run fetch-models` from jarvis folder."))
}

pub fn resolve_router_model_path(
    app: &AppHandle,
    custom_path: Option<&str>,
) -> Result<PathBuf, String> {
    resolve_model(app, custom_path, ROUTER_MODEL_FILE, "Router")
}

pub fn resolve_composer_model_path(
    app: &AppHandle,
    custom_path: Option<&str>,
) -> Result<PathBuf, String> {
    resolve_model(app, custom_path, COMPOSER_MODEL_FILE, "Composer")
}

pub fn warmup_dictation(app: AppHandle, use_gpu: bool) {
    std::thread::spawn(move || {
        let result = (|| {
            let path = resolve_model(&app, None, DICTATION_MODEL_FILE, "Dictation")?;
            let path = path.to_string_lossy().into_owned();
            let mut slot = DICTATION_SERVER.get_or_init(|| Mutex::new(None)).lock().map_err(|_| "dictation sidecar mutex poisoned".to_string())?;
            let conn = crate::open_db_connection(&app)?;
            let settings = crate::db::get_app_settings(&conn).map_err(|e| e.to_string())?;
            if !settings.dictation_cleanup || settings.local_whisper_use_gpu != use_gpu {
                return Ok(());
            }
            if slot.as_ref().is_some_and(|s| s.model_path == path && s.use_gpu == use_gpu) { return Ok(()); }
            *slot = None;
            let mut args = sidecar_args(&path, 2048, use_gpu);
            args.extend(["--temp".into(), "0".into()]);
            let sidecar = sidecar::spawn(&app, LLAMA_SERVER_EXE, &args, "/health", SIDECAR_START_TIMEOUT)?;
            *slot = Some(RouterServer { model_path: path, use_gpu, sidecar });
            Ok::<(), String>(())
        })();
        if let Err(e) = result { log::warn!("S1-mini by Superwhisper warmup: {e}"); }
    });
}

pub(crate) fn dictation_server_port(use_gpu: bool) -> Option<u16> {
    let slot = DICTATION_SERVER.get()?.try_lock().ok()?;
    slot.as_ref().filter(|server| server.use_gpu == use_gpu).map(|server| server.sidecar.port)
}

pub fn shutdown_dictation_server() {
    if let Some(slot) = DICTATION_SERVER.get() {
        if let Ok(mut server) = slot.lock() { *server = None; }
    }
}

fn sidecar_args(model_path: &str, context: u32, use_gpu: bool) -> Vec<String> {
    vec![
        "-m".into(),
        model_path.into(),
        "-c".into(),
        context.to_string(),
        "-ngl".into(),
        if use_gpu { "99" } else { "0" }.into(),
    ]
}

fn router_sidecar_args(model_path: &str, use_gpu: bool) -> Vec<String> {
    let mut args = sidecar_args(model_path, ROUTER_N_CTX, use_gpu);
    args.extend(["-b".into(), "2048".into(), "-ub".into(), "2048".into()]);
    args
}

/// Ensure the single persistent router server matches current model and acceleration settings.
pub fn ensure_router_server(
    app: &AppHandle,
    model_path: &str,
    use_gpu: bool,
) -> Result<(), String> {
    let slot = ROUTER_SERVER.get_or_init(|| Mutex::new(None));
    let mut guard = slot
        .lock()
        .map_err(|_| "router sidecar mutex poisoned".to_string())?;
    if guard
        .as_ref()
        .is_some_and(|s| s.model_path == model_path && s.use_gpu == use_gpu)
    {
        return Ok(());
    }
    *guard = None;
    let sidecar = sidecar::spawn(
        app,
        LLAMA_SERVER_EXE,
        &router_sidecar_args(model_path, use_gpu),
        "/health",
        SIDECAR_START_TIMEOUT,
    )?;
    *guard = Some(RouterServer {
        model_path: model_path.into(),
        use_gpu,
        sidecar,
    });
    Ok(())
}

pub fn shutdown_router_server() {
    if let Some(slot) = ROUTER_SERVER.get() {
        if let Ok(mut server) = slot.lock() {
            *server = None;
        }
    }
}

fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

pub(crate) fn router_server_port() -> Result<u16, String> {
    let slot = ROUTER_SERVER
        .get()
        .ok_or_else(|| "router llama-server is not running".to_string())?;
    let guard = slot
        .lock()
        .map_err(|_| "router sidecar mutex poisoned".to_string())?;
    guard
        .as_ref()
        .map(|server| server.sidecar.port)
        .ok_or_else(|| "router llama-server is not running".to_string())
}

/// Send a decision-model request to llama-server's `/v1/systemone` endpoint.
pub fn systemone(port: u16, body: &serde_json::Value) -> Result<serde_json::Value, String> {
    const SYSTEMONE_TIMEOUT: Duration = Duration::from_secs(5);
    response_json(
        post_json(port, "/v1/systemone", body, SYSTEMONE_TIMEOUT),
        "/v1/systemone",
    )
}

fn response_json(
    response: Result<ureq::Response, String>,
    endpoint: &str,
) -> Result<serde_json::Value, String> {
    let response = response.map_err(|e| format!("llama-server {endpoint} request failed: {e}"))?;
    let body = response
        .into_string()
        .map_err(|e| format!("read llama-server {endpoint} response: {e}"))?;
    serde_json::from_str(&body)
        .map_err(|e| format!("llama-server {endpoint} returned invalid JSON: {e}"))
}

fn post_json(
    port: u16,
    endpoint: &str,
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<ureq::Response, String> {
    let body = serde_json::to_string(body).expect("JSON value serialization cannot fail");
    ureq::AgentBuilder::new()
        .timeout(timeout)
        .build()
        .post(&format!("{}{endpoint}", base_url(port)))
        .set("Content-Type", "application/json")
    .send_string(&body)
    .map_err(|e| e.to_string())
}

fn render_prompt(
    port: u16,
    system: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String, String> {
    let body = serde_json::json!({ "messages": [
        { "role": "system", "content": system },
        { "role": "user", "content": prompt }
    ], "add_generation_prompt": true});
    let result = response_json(
        post_json(port, "/apply-template", &body, timeout),
        "/apply-template",
    )?;
    result
        .get("prompt")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "llama-server /apply-template response has no prompt".into())
}

fn complete(
    port: u16,
    system: &str,
    prompt: &str,
    n_predict: i32,
    cancel: Option<&AtomicBool>,
    json_schema: Option<&serde_json::Value>,
    timeout: Duration,
) -> Result<String, String> {
    if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
        return Err("composer inference cancelled".into());
    }
    let prompt = render_prompt(port, system, prompt, timeout)?;
    let mut body = serde_json::json!({ "prompt": prompt, "n_predict": n_predict, "temperature": 0.1, "seed": 42, "top_k": 1, "cache_prompt": true });
    if let Some(schema) = json_schema {
        body["json_schema"] = schema.clone();
    }
    if cancel.is_some() {
        body["stream"] = serde_json::Value::Bool(true);
    }
    let response = post_json(port, "/completion", &body, timeout)
        .map_err(|e| format!("llama-server /completion request failed: {e}"))?;
    let result = if let Some(cancel) = cancel {
        let mut reader = BufReader::new(response.into_reader());
        let mut out = String::new();
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err("composer inference cancelled".into());
            }
            let mut line = String::new();
            if reader
                .read_line(&mut line)
                .map_err(|e| format!("read llama-server completion stream: {e}"))?
                == 0
            {
                break;
            }
            let Some(data) = line.trim().strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                break;
            }
            let chunk: serde_json::Value = serde_json::from_str(data)
                .map_err(|e| format!("invalid llama-server completion chunk: {e}"))?;
            if let Some(content) = chunk.get("content").and_then(serde_json::Value::as_str) {
                out.push_str(content);
            }
            if chunk.get("stop").and_then(serde_json::Value::as_bool) == Some(true) {
                break;
            }
        }
        out
    } else {
        let body = response
            .into_string()
            .map_err(|e| format!("read llama-server completion response: {e}"))?;
        let result: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| format!("llama-server /completion returned invalid JSON: {e}"))?;
        result
            .get("content")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "llama-server /completion response has no content".to_string())?
    };
    if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
        return Err("composer inference cancelled".into());
    }
    Ok(result)
}

/// Public compatibility signature; the router sidecar is started by router warmup first.
pub fn infer_router_completion(
    _model_path: &str,
    prompt: &str,
    _use_gpu: bool,
    json_schema: Option<&serde_json::Value>,
) -> Result<String, String> {
    let app = APP_HANDLE
        .get()
        .ok_or_else(|| "Tauri app is not initialized".to_string())?;
    ensure_router_server(app, _model_path, _use_gpu)?;
    let slot = ROUTER_SERVER
        .get()
        .ok_or_else(|| "router llama-server is not running".to_string())?;
    let guard = slot
        .lock()
        .map_err(|_| "router sidecar mutex poisoned".to_string())?;
    let server = guard
        .as_ref()
        .ok_or_else(|| "router llama-server is not running".to_string())?;
    complete(
        server.sidecar.port,
        ROUTER_SYSTEM_PROMPT,
        prompt,
        MAX_GENERATED_TOKENS,
        None,
        json_schema,
        COMPOSER_INFER_TIMEOUT,
    )
}

struct ComposerInferRequest {
    prompt: String,
    max_tokens: i32,
    cancel: Arc<AtomicBool>,
    respond_to: mpsc::Sender<Result<String, String>>,
}

/// A persistent composer sidecar and request worker.
pub struct ComposerInferWorker {
    sender: mpsc::Sender<ComposerInferRequest>,
    _sidecar: Sidecar,
}

impl ComposerInferWorker {
    pub fn spawn(model_path: String, use_gpu: bool) -> Result<Self, String> {
        let app = APP_HANDLE
            .get()
            .ok_or_else(|| "Tauri app is not initialized".to_string())?;
        Self::spawn_with_app(app, model_path, use_gpu)
    }

    pub fn spawn_with_app(app: &AppHandle, model_path: String, use_gpu: bool) -> Result<Self, String> {
        let sidecar = sidecar::spawn(
            app,
            LLAMA_SERVER_EXE,
            &sidecar_args(&model_path, COMPOSER_N_CTX, use_gpu),
            "/health",
            SIDECAR_START_TIMEOUT,
        )?;
        Self::from_sidecar(sidecar)
    }

    #[cfg(test)]
    pub fn spawn_with_resource_root(
        resource_root: &std::path::Path,
        model_path: String,
        use_gpu: bool,
    ) -> Result<Self, String> {
        let sidecar = sidecar::spawn_at(
            resource_root,
            LLAMA_SERVER_EXE,
            &sidecar_args(&model_path, COMPOSER_N_CTX, use_gpu),
            "/health",
            SIDECAR_START_TIMEOUT,
        )?;
        Self::from_sidecar(sidecar)
    }

    fn from_sidecar(sidecar: Sidecar) -> Result<Self, String> {
        let port = sidecar.port;
        let (tx, rx) = mpsc::channel::<ComposerInferRequest>();
        std::thread::Builder::new()
            .name("composer-http-worker".into())
            .spawn(move || {
                while let Ok(req) = rx.recv() {
                    let result = complete(
                        port,
                        "JSON. JSON only.",
                        &req.prompt,
                        req.max_tokens.clamp(1, COMPOSER_MAX_MAX_TOKENS),
                        Some(&req.cancel),
                        None,
                        COMPOSER_INFER_TIMEOUT,
                    );
                    let _ = req.respond_to.send(result);
                }
            })
            .map_err(|e| format!("spawn composer inference worker: {e}"))?;
        Ok(Self {
            sender: tx,
            _sidecar: sidecar,
        })
    }

    pub fn infer(
        &self,
        prompt: &str,
        max_tokens: i32,
        cancel: Arc<AtomicBool>,
    ) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        self.sender
            .send(ComposerInferRequest {
                prompt: prompt.into(),
                max_tokens,
                cancel,
                respond_to: tx,
            })
            .map_err(|_| "composer inference worker stopped".to_string())?;
        rx.recv_timeout(COMPOSER_INFER_TIMEOUT)
            .map_err(|e| match e {
                mpsc::RecvTimeoutError::Timeout => {
                    "composer inference timed out after 90 seconds".to_string()
                }
                mpsc::RecvTimeoutError::Disconnected => {
                    "composer inference worker stopped".to_string()
                }
            })?
    }
}

/// Router eval: start a standalone router sidecar for `model_path` (no AppHandle).
#[cfg(test)]
pub(crate) fn spawn_router_sidecar_at(
    resource_root: &std::path::Path,
    model_path: &str,
    use_gpu: bool,
) -> Result<Sidecar, String> {
    sidecar::spawn_at(
        resource_root,
        LLAMA_SERVER_EXE,
        &router_sidecar_args(model_path, use_gpu),
        "/health",
        SIDECAR_START_TIMEOUT,
    )
}

/// Router eval: same request the live router sends, against an eval sidecar.
#[cfg(test)]
pub(crate) fn complete_router_at(
    port: u16,
    prompt: &str,
    json_schema: &serde_json::Value,
) -> Result<String, String> {
    complete(
        port,
        ROUTER_SYSTEM_PROMPT,
        prompt,
        MAX_GENERATED_TOKENS,
        None,
        Some(json_schema),
        COMPOSER_INFER_TIMEOUT,
    )
}

/// Router eval, decision mode: one forced token from `A..` (n options); returns the model's
/// probability per option, renormalized over the valid letters.
#[cfg(test)]
pub(crate) fn choose_option_probs_at(
    port: u16,
    system: &str,
    prompt: &str,
    n_options: usize,
) -> Result<Vec<f64>, String> {
    let letters: Vec<char> = ('A'..='Z').take(n_options).collect();
    let prompt = render_prompt(port, system, prompt, COMPOSER_INFER_TIMEOUT)?;
    let grammar = format!(
        "root ::= [{}]",
        letters.iter().collect::<String>()
    );
    let body = serde_json::json!({
        "prompt": prompt, "n_predict": 1, "temperature": 0.0, "top_k": 1,
        "cache_prompt": true, "grammar": grammar, "n_probs": 40
    });
    let result = response_json(
        post_json(port, "/completion", &body, COMPOSER_INFER_TIMEOUT),
        "/completion",
    )?;
    let first = result
        .get("completion_probabilities")
        .and_then(|p| p.get(0))
        .ok_or("no completion_probabilities (n_probs unsupported?)")?;
    // Newer llama-server: top_logprobs[{token, logprob}]; older: probs[{tok_str, prob}].
    let entries: Vec<(String, f64)> = if let Some(top) = first.get("top_logprobs").and_then(|v| v.as_array()) {
        top.iter()
            .filter_map(|e| Some((e.get("token")?.as_str()?.to_string(), e.get("logprob")?.as_f64()?.exp())))
            .collect()
    } else {
        first
            .get("probs")
            .and_then(|v| v.as_array())
            .ok_or("unrecognized completion_probabilities shape")?
            .iter()
            .filter_map(|e| Some((e.get("tok_str")?.as_str()?.to_string(), e.get("prob")?.as_f64()?)))
            .collect()
    };
    let mut probs = vec![0.0; n_options];
    for (token, p) in entries {
        let t = token.trim();
        if t.len() == 1 {
            if let Some(i) = letters.iter().position(|c| t.starts_with(*c)) {
                probs[i] += p;
            }
        }
    }
    let total: f64 = probs.iter().sum();
    if total <= 0.0 {
        return Err("no option letters among top tokens".into());
    }
    Ok(probs.iter().map(|p| p / total).collect())
}

pub fn infer_composer_completion(
    worker: &ComposerInferWorker,
    prompt: &str,
    max_tokens: i32,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    worker.infer(prompt, max_tokens, cancel)
}

#[cfg(test)]
mod router_args_tests {
    use super::{router_sidecar_args, sidecar_args, COMPOSER_N_CTX};

    #[test]
    fn batching_args_are_router_only() {
        let router = router_sidecar_args("d1.gguf", false);
        let composer = sidecar_args("composer.gguf", COMPOSER_N_CTX, false);
        assert!(router.windows(2).any(|args| args == ["-b", "2048"]));
        assert!(router.windows(2).any(|args| args == ["-ub", "2048"]));
        assert!(!composer.iter().any(|arg| arg == "-b" || arg == "-ub"));
    }
}

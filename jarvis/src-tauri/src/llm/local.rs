//! GGUF load + inference via llama.cpp (`llm-local` feature).

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

use encoding_rs::UTF_8;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use tauri::{AppHandle, Manager};

pub const ROUTER_MODEL_FILE: &str = "qwen2.5-0.5b-instruct-q4_k_m.gguf";
pub const COMPOSER_MODEL_FILE: &str = "qwen2.5-3b-instruct-q4_k_m.gguf";
/// llama.cpp context init/decode/teardown uses deep native stacks on Windows MSVC.
pub const ROUTER_LOADER_STACK: usize = crate::gpu_startup::WHISPER_LOADER_STACK;
const MAX_GENERATED_TOKENS: i32 = 256;
const ROUTER_N_CTX: u32 = 2048;
const COMPOSER_N_CTX: u32 = 4096;
pub const COMPOSER_DEFAULT_MAX_TOKENS: i32 = 512;
pub const COMPOSER_MAX_MAX_TOKENS: i32 = 768;

static LLAMA_BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();
static ROUTER_LOAD_GATE: Mutex<()> = Mutex::new(());

fn backend() -> Result<&'static LlamaBackend, String> {
    LLAMA_BACKEND
        .get_or_init(|| LlamaBackend::init().map_err(|e| format!("llama backend init failed: {e}")))
        .as_ref()
        .map_err(|e| e.clone())
}

pub fn llm_compile_backend() -> &'static str {
    if cfg!(feature = "llm-metal") {
        "metal"
    } else if cfg!(feature = "llm-cuda") {
        "cuda"
    } else if cfg!(feature = "llm-vulkan") {
        "vulkan"
    } else {
        "none"
    }
}

#[cfg(target_os = "windows")]
fn try_load_library(lib_name: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::System::LibraryLoader::LoadLibraryW;

    let mut wide: Vec<u16> = lib_name.encode_utf16().collect();
    wide.push(0);
    // SAFETY: null-terminated UTF-16 module name for loader probe.
    unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())).is_ok() }
}

pub fn llm_gpu_runtime_available(backend: &str) -> bool {
    match backend {
        "metal" => cfg!(target_os = "macos"),
        "cuda" => llm_cuda_runtime_available(),
        "vulkan" => llm_vulkan_runtime_available(),
        _ => false,
    }
}

fn llm_cuda_runtime_available() -> bool {
    #[cfg(target_os = "windows")]
    {
        return try_load_library("nvcuda.dll");
    }
    #[cfg(target_os = "linux")]
    {
        return std::path::Path::new("/usr/lib/x86_64-linux-gnu/libcuda.so.1").exists()
            || std::path::Path::new("/usr/lib64/libcuda.so.1").exists()
            || std::path::Path::new("/usr/lib/libcuda.so.1").exists();
    }
    #[cfg(target_os = "macos")]
    {
        return false;
    }
    #[allow(unreachable_code)]
    false
}

fn llm_vulkan_runtime_available() -> bool {
    #[cfg(target_os = "windows")]
    {
        return try_load_library("vulkan-1.dll");
    }
    #[cfg(target_os = "linux")]
    {
        return std::path::Path::new("/usr/lib/x86_64-linux-gnu/libvulkan.so.1").exists()
            || std::path::Path::new("/usr/lib64/libvulkan.so.1").exists()
            || std::path::Path::new("/usr/lib/libvulkan.so.1").exists();
    }
    #[cfg(target_os = "macos")]
    {
        return std::path::Path::new("/usr/local/lib/libvulkan.dylib").exists()
            || std::path::Path::new("/opt/homebrew/lib/libvulkan.dylib").exists();
    }
    #[allow(unreachable_code)]
    false
}

fn router_model_candidates(app: &AppHandle, custom_path: Option<&str>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(path) = custom_path.map(str::trim).filter(|p| !p.is_empty()) {
        out.push(PathBuf::from(path));
    }
    if let Ok(dir) = app.path().resource_dir() {
        out.push(dir.join(ROUTER_MODEL_FILE));
    }
    out.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(ROUTER_MODEL_FILE),
    );
    out
}

pub fn resolve_router_model_path(app: &AppHandle, custom_path: Option<&str>) -> Result<PathBuf, String> {
    let candidates = router_model_candidates(app, custom_path);
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
        "Router model `{ROUTER_MODEL_FILE}` not found (tried: {tried}). Run `npm run fetch-models` from the jarvis folder."
    ))
}

fn model_params(use_gpu: bool) -> LlamaModelParams {
    #[cfg(any(feature = "llm-cuda", feature = "llm-vulkan", feature = "llm-metal"))]
    {
        if use_gpu {
            return LlamaModelParams::default().with_n_gpu_layers(1000);
        }
    }
    let _ = use_gpu;
    LlamaModelParams::default()
}

fn build_prompt(model: &LlamaModel, prompt: &str) -> Result<String, String> {
    let tmpl = model
        .chat_template(None)
        .map_err(|e| format!("chat template missing: {e}"))?;
    let messages = [
        LlamaChatMessage::new("system".into(), "You route voice commands to JSON tool calls.".into())
            .map_err(|e| format!("chat message error: {e}"))?,
        LlamaChatMessage::new("user".into(), prompt.to_string())
            .map_err(|e| format!("chat message error: {e}"))?,
    ];
    model
        .apply_chat_template(&tmpl, &messages, true)
        .map_err(|e| format!("apply chat template failed: {e}"))
}

fn generate_completion(model: &LlamaModel, prompt: &str) -> Result<String, String> {
    let backend = backend()?;
    let mut ctx = model
        .new_context(
            backend,
            LlamaContextParams::default().with_n_ctx(Some(NonZeroU32::new(ROUTER_N_CTX).unwrap())),
        )
        .map_err(|e| format!("create llama context failed: {e}"))?;

    let full_prompt = build_prompt(model, prompt)?;
    let tokens = model
        .str_to_token(&full_prompt, AddBos::Always)
        .map_err(|e| format!("tokenize failed: {e}"))?;

    let mut batch = LlamaBatch::new(ROUTER_N_CTX as usize, 1);
    let last_index = tokens.len().saturating_sub(1) as i32;
    for (i, token) in tokens.into_iter().enumerate() {
        let is_last = i as i32 == last_index;
        batch
            .add(token, i as i32, &[0], is_last)
            .map_err(|e| format!("batch add failed: {e}"))?;
    }
    ctx.decode(&mut batch)
        .map_err(|e| format!("llama decode failed: {e}"))?;

    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::temp(0.1),
        LlamaSampler::dist(42),
        LlamaSampler::greedy(),
    ]);
    let mut decoder = UTF_8.new_decoder();
    let mut out = String::new();
    let mut n_cur = batch.n_tokens();

    for _ in 0..MAX_GENERATED_TOKENS {
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);
        if model.is_eog_token(token) {
            break;
        }
        let piece = model
            .token_to_piece(token, &mut decoder, true, None)
            .map_err(|e| format!("token decode failed: {e}"))?;
        out.push_str(&piece);

        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .map_err(|e| format!("batch add failed: {e}"))?;
        n_cur += 1;
        ctx.decode(&mut batch)
            .map_err(|e| format!("llama decode failed: {e}"))?;
    }

    Ok(out)
}

fn infer_router_completion_inner(
    model_path: &str,
    prompt: &str,
    use_gpu: bool,
) -> Result<String, String> {
    let _gate = ROUTER_LOAD_GATE
        .lock()
        .map_err(|_| "router load mutex poisoned".to_string())?;
    let use_accel = use_gpu && llm_gpu_runtime_available(llm_compile_backend());

    let load_and_infer = || -> Result<String, String> {
        let backend = backend()?;
        let path = Path::new(model_path);
        let params = pin!(model_params(use_gpu));
        let model = LlamaModel::load_from_file(backend, path, &params)
            .map_err(|e| format!("load router model failed: {e}"))?;
        match generate_completion(&model, prompt) {
            Ok(text) => Ok(text),
            Err(gpu_err) if use_accel => {
                log::warn!("router gpu infer failed; retrying on cpu: {gpu_err}");
                generate_completion(&model, prompt)
            }
            Err(e) => Err(e),
        }
    };

    if use_accel {
        crate::gpu_startup::with_ggml_cuda_init(true, load_and_infer)
    } else {
        load_and_infer()
    }
}

pub fn infer_router_completion(model_path: &str, prompt: &str, use_gpu: bool) -> Result<String, String> {
    let model_path = model_path.to_string();
    let prompt = prompt.to_string();
    std::thread::Builder::new()
        .name("router-infer".into())
        .stack_size(ROUTER_LOADER_STACK)
        .spawn(move || infer_router_completion_inner(&model_path, &prompt, use_gpu))
        .map_err(|e| format!("spawn router infer thread: {e}"))?
        .join()
        .map_err(|_| "router infer thread panicked".to_string())?
}

fn composer_model_candidates(app: &AppHandle, custom_path: Option<&str>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(path) = custom_path.map(str::trim).filter(|p| !p.is_empty()) {
        out.push(PathBuf::from(path));
    }
    if let Ok(dir) = app.path().resource_dir() {
        out.push(dir.join(COMPOSER_MODEL_FILE));
    }
    out.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(COMPOSER_MODEL_FILE),
    );
    out
}

pub fn resolve_composer_model_path(app: &AppHandle, custom_path: Option<&str>) -> Result<PathBuf, String> {
    let candidates = composer_model_candidates(app, custom_path);
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
        "Composer model `{COMPOSER_MODEL_FILE}` not found (tried: {tried}). Run `npm run fetch-models` from the jarvis folder."
    ))
}

fn build_composer_chat_prompt(model: &LlamaModel, prompt: &str) -> Result<String, String> {
    let tmpl = model
        .chat_template(None)
        .map_err(|e| format!("chat template missing: {e}"))?;
    let messages = [
        LlamaChatMessage::new(
            "system".into(),
            "You compose voice automations as strict JSON. Return one JSON object only.".into(),
        )
        .map_err(|e| format!("chat message error: {e}"))?,
        LlamaChatMessage::new("user".into(), prompt.to_string())
            .map_err(|e| format!("chat message error: {e}"))?,
    ];
    model
        .apply_chat_template(&tmpl, &messages, true)
        .map_err(|e| format!("apply chat template failed: {e}"))
}

fn generate_composer_completion(
    model: &LlamaModel,
    prompt: &str,
    max_tokens: i32,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let backend = backend()?;
    let mut ctx = model
        .new_context(
            backend,
            LlamaContextParams::default().with_n_ctx(Some(NonZeroU32::new(COMPOSER_N_CTX).unwrap())),
        )
        .map_err(|e| format!("create llama context failed: {e}"))?;

    let full_prompt = build_composer_chat_prompt(model, prompt)?;
    let tokens = model
        .str_to_token(&full_prompt, AddBos::Always)
        .map_err(|e| format!("tokenize failed: {e}"))?;
    if tokens.len() > COMPOSER_N_CTX as usize {
        return Err(format!(
            "composer prompt is {} tokens; max context is {COMPOSER_N_CTX}",
            tokens.len()
        ));
    }

    let mut batch = LlamaBatch::new(COMPOSER_N_CTX as usize, 1);
    let last_index = tokens.len().saturating_sub(1) as i32;
    for (i, token) in tokens.into_iter().enumerate() {
        let is_last = i as i32 == last_index;
        batch
            .add(token, i as i32, &[0], is_last)
            .map_err(|e| format!("batch add failed: {e}"))?;
    }
    ctx.decode(&mut batch)
        .map_err(|e| format!("llama decode failed: {e}"))?;

    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::temp(0.1),
        LlamaSampler::dist(42),
        LlamaSampler::greedy(),
    ]);
    let mut decoder = UTF_8.new_decoder();
    let mut out = String::new();
    let mut n_cur = batch.n_tokens();
    let token_budget = max_tokens.clamp(1, COMPOSER_MAX_MAX_TOKENS);

    for _ in 0..token_budget {
        if cancel.load(Ordering::SeqCst) {
            return Err("composer inference cancelled".into());
        }
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);
        if model.is_eog_token(token) {
            break;
        }
        let piece = model
            .token_to_piece(token, &mut decoder, true, None)
            .map_err(|e| format!("token decode failed: {e}"))?;
        out.push_str(&piece);

        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .map_err(|e| format!("batch add failed: {e}"))?;
        n_cur += 1;
        ctx.decode(&mut batch)
            .map_err(|e| format!("llama decode failed: {e}"))?;
    }

    Ok(out)
}

fn load_composer_model(model_path: &str, use_gpu: bool) -> Result<LlamaModel, String> {
    let backend = backend()?;
    let path = Path::new(model_path);
    let params = pin!(model_params(use_gpu));
    let load = || {
        LlamaModel::load_from_file(backend, path, &params)
            .map_err(|e| format!("load composer model failed: {e}"))
    };
    let use_accel = use_gpu && llm_gpu_runtime_available(llm_compile_backend());
    if use_accel {
        crate::gpu_startup::with_ggml_cuda_init(true, load)
    } else {
        load()
    }
}

struct ComposerInferRequest {
    prompt: String,
    max_tokens: i32,
    cancel: Arc<AtomicBool>,
    respond_to: mpsc::Sender<Result<String, String>>,
}

/// Long-lived composer inference worker — owns one loaded `LlamaModel` on a pinned thread.
pub struct ComposerInferWorker {
    sender: mpsc::Sender<ComposerInferRequest>,
}

impl ComposerInferWorker {
    pub fn spawn(model_path: String, use_gpu: bool) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let path_for_thread = model_path.clone();
        std::thread::Builder::new()
            .name("composer-worker".into())
            .stack_size(ROUTER_LOADER_STACK)
            .spawn(move || composer_worker_loop(rx, path_for_thread, use_gpu))
            .map_err(|e| format!("spawn composer worker: {e}"))?;
        Ok(Self { sender: tx })
    }

    pub fn infer(
        &self,
        prompt: &str,
        max_tokens: i32,
        cancel: Arc<AtomicBool>,
    ) -> Result<String, String> {
        let (respond_tx, respond_rx) = mpsc::channel();
        self.sender
            .send(ComposerInferRequest {
                prompt: prompt.to_string(),
                max_tokens,
                cancel,
                respond_to: respond_tx,
            })
            .map_err(|_| "composer worker channel closed".to_string())?;
        respond_rx
            .recv()
            .map_err(|_| "composer worker dropped response".to_string())?
    }
}

fn composer_worker_loop(
    rx: mpsc::Receiver<ComposerInferRequest>,
    model_path: String,
    use_gpu: bool,
) {
    let mut model: Option<LlamaModel> = None;
    while let Ok(req) = rx.recv() {
        if req.cancel.load(Ordering::SeqCst) {
            let _ = req
                .respond_to
                .send(Err("composer inference cancelled".into()));
            continue;
        }
        if model.is_none() {
            match load_composer_model(&model_path, use_gpu) {
                Ok(loaded) => model = Some(loaded),
                Err(e) => {
                    let _ = req.respond_to.send(Err(e));
                    continue;
                }
            }
        }
        let model_ref = model.as_ref().expect("model loaded");
        let use_accel = use_gpu && llm_gpu_runtime_available(llm_compile_backend());
        let result = match generate_composer_completion(
            model_ref,
            &req.prompt,
            req.max_tokens,
            &req.cancel,
        ) {
            Ok(text) => Ok(text),
            Err(gpu_err) if use_accel => {
                log::warn!("composer gpu infer failed; retrying on cpu: {gpu_err}");
                generate_composer_completion(model_ref, &req.prompt, req.max_tokens, &req.cancel)
            }
            Err(e) => Err(e),
        };
        let _ = req.respond_to.send(result);
    }
}

/// Blocking composer inference via the resident worker (caller should be off the UI thread).
pub fn infer_composer_completion(
    worker: &ComposerInferWorker,
    prompt: &str,
    max_tokens: i32,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    worker.infer(prompt, max_tokens, cancel)
}

pub const COMPOSER_INFER_TIMEOUT: Duration = Duration::from_secs(90);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_compile_backend_is_known() {
        assert!(matches!(
            llm_compile_backend(),
            "none" | "vulkan" | "cuda" | "metal"
        ));
    }
}

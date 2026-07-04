//! Whisper STT model catalog, user-managed storage, and Hugging Face downloads.

use crate::audio::stt::{parse_local_whisper_model_id, LOCAL_WHISPER_MODEL_IDS};
use log::{debug, info, warn};
use serde::Serialize;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

pub const WHISPER_HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

#[derive(Debug, Clone, Copy)]
pub struct WhisperModelSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub size_bytes_approx: u64,
}

pub const WHISPER_MODEL_SPECS: &[WhisperModelSpec] = &[
    WhisperModelSpec {
        id: "tiny.en",
        label: "Tiny",
        size_bytes_approx: 75 * 1024 * 1024,
    },
    WhisperModelSpec {
        id: "base.en",
        label: "Base",
        size_bytes_approx: 142 * 1024 * 1024,
    },
    WhisperModelSpec {
        id: "small.en",
        label: "Small",
        size_bytes_approx: 466 * 1024 * 1024,
    },
];

pub fn whisper_model_filename(model_id: &str) -> String {
    format!("ggml-{}.bin", parse_local_whisper_model_id(Some(model_id)))
}

pub fn whisper_models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("whisper-models");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub fn managed_model_path(app: &AppHandle, model_id: &str) -> Result<PathBuf, String> {
    Ok(whisper_models_dir(app)?.join(whisper_model_filename(model_id)))
}

fn bundled_model_candidates(app: &AppHandle, model_id: &str) -> Vec<PathBuf> {
    let filename = whisper_model_filename(model_id);
    let mut out = Vec::new();
    if let Ok(dir) = app.path().resource_dir() {
        out.push(dir.join(&filename));
    }
    out.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(filename),
    );
    out
}

fn file_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|m| m.len())
}

fn is_usable_model_file(path: &Path) -> bool {
    file_size(path).is_some_and(|n| n > 1024)
}

/// Resolve an on-disk Whisper weights file: user-managed dir first, then bundled resources.
pub fn resolve_whisper_model_path_for_id(
    app: &AppHandle,
    model_id: &str,
) -> Result<PathBuf, String> {
    let model_id = parse_local_whisper_model_id(Some(model_id));
    let filename = whisper_model_filename(model_id);

    if let Ok(managed) = managed_model_path(app, model_id) {
        if is_usable_model_file(&managed) {
            return Ok(managed);
        }
    }

    for path in bundled_model_candidates(app, model_id) {
        if is_usable_model_file(&path) {
            return Ok(path);
        }
    }

    let mut tried = Vec::new();
    if let Ok(managed) = managed_model_path(app, model_id) {
        tried.push(managed.display().to_string());
    }
    for path in bundled_model_candidates(app, model_id) {
        tried.push(path.display().to_string());
    }
    Err(format!(
        "Whisper model `{filename}` not found (tried: {}). Download it from Settings → Speech.",
        tried.join(", ")
    ))
}

pub fn is_whisper_model_installed(app: &AppHandle, model_id: &str) -> bool {
    resolve_whisper_model_path_for_id(app, model_id).is_ok()
}

fn spec_for_id(model_id: &str) -> Option<&'static WhisperModelSpec> {
    let id = parse_local_whisper_model_id(Some(model_id));
    WHISPER_MODEL_SPECS.iter().find(|s| s.id == id)
}

fn download_url(model_id: &str) -> Option<String> {
    spec_for_id(model_id).map(|_| {
        format!(
            "{}/{}",
            WHISPER_HF_BASE,
            whisper_model_filename(model_id)
        )
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhisperModelEntryPayload {
    pub id: String,
    pub label: String,
    pub installed: bool,
    pub size_bytes: Option<u64>,
    pub size_bytes_approx: u64,
    pub active: bool,
    pub downloading: bool,
    pub download_progress_pct: Option<u8>,
    pub managed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListWhisperModelsPayload {
    pub models: Vec<WhisperModelEntryPayload>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhisperModelDownloadEvent {
    pub model_id: String,
    pub phase: String,
    pub progress_pct: Option<u8>,
    pub message: Option<String>,
}

pub struct WhisperModelDownloadState(Mutex<Option<ActiveDownload>>);

#[derive(Debug, Clone)]
struct ActiveDownload {
    model_id: String,
    progress_pct: u8,
}

impl Default for WhisperModelDownloadState {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}

impl WhisperModelDownloadState {
    fn snapshot(&self, model_id: &str) -> (bool, Option<u8>) {
        let guard = self.0.lock().ok();
        match guard.as_ref().and_then(|g| g.as_ref()) {
            Some(active) if active.model_id == model_id => {
                (true, Some(active.progress_pct))
            }
            _ => (false, None),
        }
    }

    fn try_begin(&self, model_id: &str) -> Result<(), String> {
        let mut guard = self
            .0
            .lock()
            .map_err(|_| "whisper download state poisoned".to_string())?;
        if let Some(active) = guard.as_ref() {
            if active.model_id == model_id {
                return Ok(());
            }
            return Err(format!(
                "Already downloading `{}`. Wait for it to finish.",
                active.model_id
            ));
        }
        *guard = Some(ActiveDownload {
            model_id: model_id.to_string(),
            progress_pct: 0,
        });
        Ok(())
    }

    fn set_progress(&self, model_id: &str, pct: u8) {
        if let Ok(mut guard) = self.0.lock() {
            if let Some(active) = guard.as_mut() {
                if active.model_id == model_id {
                    active.progress_pct = pct;
                }
            }
        }
    }

    fn finish(&self, model_id: &str) {
        if let Ok(mut guard) = self.0.lock() {
            if guard
                .as_ref()
                .is_some_and(|a| a.model_id == model_id)
            {
                *guard = None;
            }
        }
    }
}

fn emit_download_event(app: &AppHandle, event: WhisperModelDownloadEvent) {
    let _ = app.emit("whisper-model-download", event);
}

fn active_model_id(app: &AppHandle) -> String {
    crate::open_db_connection(app)
        .ok()
        .and_then(|conn| crate::db::get_app_settings(&conn).ok())
        .map(|s| s.local_whisper_model)
        .unwrap_or_else(|| crate::audio::stt::DEFAULT_LOCAL_WHISPER_MODEL.to_string())
}

pub fn list_whisper_models(app: &AppHandle) -> Result<ListWhisperModelsPayload, String> {
    let download_state = app.state::<WhisperModelDownloadState>();
    let active = active_model_id(app);
    let mut models = Vec::with_capacity(LOCAL_WHISPER_MODEL_IDS.len());

    for &id in LOCAL_WHISPER_MODEL_IDS {
        let spec = spec_for_id(id).expect("catalog covers LOCAL_WHISPER_MODEL_IDS");
        let managed_path = managed_model_path(app, id).ok();
        let managed_installed = managed_path
            .as_ref()
            .is_some_and(|p| is_usable_model_file(p));
        let installed = is_whisper_model_installed(app, id);
        let size_bytes = if managed_installed {
            managed_path.as_ref().and_then(|p| file_size(p))
        } else if installed {
            resolve_whisper_model_path_for_id(app, id)
                .ok()
                .and_then(|p| file_size(&p))
        } else {
            None
        };
        let (downloading, download_progress_pct) = download_state.snapshot(id);
        models.push(WhisperModelEntryPayload {
            id: id.to_string(),
            label: spec.label.to_string(),
            installed,
            size_bytes,
            size_bytes_approx: spec.size_bytes_approx,
            active: active == id,
            downloading,
            download_progress_pct,
            managed: managed_installed,
        });
    }

    Ok(ListWhisperModelsPayload { models })
}

fn download_whisper_model_to_path(
    app: &AppHandle,
    model_id: &str,
    url: &str,
    dest: &Path,
) -> Result<(), String> {
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(3600))
        .call()
        .map_err(|e| format!("download request failed: {e}"))?;

    if resp.status() != 200 {
        return Err(format!("download HTTP {}", resp.status()));
    }

    let total = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());

    let tmp = dest.with_extension("bin.part");
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }

    let mut file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded = 0u64;
    let mut last_emit_pct = 0u8;
    let download_state = app.state::<WhisperModelDownloadState>();

    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;

        if let Some(total) = total {
            let pct = ((downloaded.saturating_mul(100)) / total.max(1)).min(100) as u8;
            if pct >= last_emit_pct.saturating_add(2) || pct == 100 {
                last_emit_pct = pct;
                download_state.set_progress(model_id, pct);
                emit_download_event(
                    app,
                    WhisperModelDownloadEvent {
                        model_id: model_id.to_string(),
                        phase: "progress".into(),
                        progress_pct: Some(pct),
                        message: None,
                    },
                );
            }
        }
    }

    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);

    if let Some(total) = total {
        let on_disk = file_size(&tmp).unwrap_or(0);
        if on_disk + 1024 < total {
            let _ = fs::remove_file(&tmp);
            return Err(format!(
                "download incomplete ({on_disk} of {total} bytes)"
            ));
        }
    }

    if dest.exists() {
        fs::remove_file(dest).map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// Download `model_id` into the user-managed models directory (blocking).
pub fn download_whisper_model_blocking(app: &AppHandle, model_id: &str) -> Result<(), String> {
    let model_id = parse_local_whisper_model_id(Some(model_id));
    if is_whisper_model_installed(app, model_id) {
        return Ok(());
    }

    let url = download_url(model_id)
        .ok_or_else(|| format!("unknown whisper model `{model_id}`"))?;
    let dest = managed_model_path(app, model_id)?;
    let download_state = app.state::<WhisperModelDownloadState>();
    download_state.try_begin(model_id)?;

    emit_download_event(
        app,
        WhisperModelDownloadEvent {
            model_id: model_id.to_string(),
            phase: "started".into(),
            progress_pct: Some(0),
            message: Some(format!("Downloading {model_id}…")),
        },
    );

    info!("whisper-models: downloading `{model_id}`");
    let result = download_whisper_model_to_path(app, model_id, &url, &dest);
    download_state.finish(model_id);

    match result {
        Ok(()) => {
            info!("whisper-models: saved `{model_id}` to {}", dest.display());
            emit_download_event(
                app,
                WhisperModelDownloadEvent {
                    model_id: model_id.to_string(),
                    phase: "done".into(),
                    progress_pct: Some(100),
                    message: Some(format!("{model_id} ready.")),
                },
            );
            Ok(())
        }
        Err(e) => {
            warn!("whisper-models: download `{model_id}` failed: {e}");
            let _ = fs::remove_file(dest.with_extension("bin.part"));
            emit_download_event(
                app,
                WhisperModelDownloadEvent {
                    model_id: model_id.to_string(),
                    phase: "error".into(),
                    progress_pct: None,
                    message: Some(e.clone()),
                },
            );
            Err(e)
        }
    }
}

/// Ensure the requested model exists locally, downloading into app data when missing.
pub fn ensure_whisper_model(app: &AppHandle, model_id: &str) -> Result<PathBuf, String> {
    let model_id = parse_local_whisper_model_id(Some(model_id));
    if let Ok(path) = resolve_whisper_model_path_for_id(app, model_id) {
        return Ok(path);
    }
    debug!("whisper-models: `{model_id}` missing; downloading");
    download_whisper_model_blocking(app, model_id)?;
    resolve_whisper_model_path_for_id(app, model_id)
}

pub fn spawn_whisper_model_download(app: AppHandle, model_id: String) -> Result<(), String> {
    let model_id_norm = parse_local_whisper_model_id(Some(model_id.as_str())).to_string();
    app.state::<WhisperModelDownloadState>()
        .try_begin(&model_id_norm)?;

    let app_bg = app.clone();
    std::thread::Builder::new()
        .name(format!("whisper-dl-{model_id_norm}"))
        .spawn(move || {
            let _ = download_whisper_model_blocking(&app_bg, &model_id_norm);
        })
        .map_err(|e| format!("failed to start download thread: {e}"))?;
    Ok(())
}

pub fn delete_whisper_model(app: &AppHandle, model_id: &str) -> Result<(), String> {
    let model_id = parse_local_whisper_model_id(Some(model_id));
    if active_model_id(app) == model_id {
        return Err(format!(
            "Cannot delete `{model_id}` while it is the active speech model. Select another model first."
        ));
    }
    let path = managed_model_path(app, model_id)?;
    if !path.is_file() {
        return Err(format!(
            "`{model_id}` is not installed in your managed models folder."
        ));
    }
    fs::remove_file(&path).map_err(|e| e.to_string())?;
    info!("whisper-models: deleted `{model_id}`");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_local_model_ids() {
        for &id in LOCAL_WHISPER_MODEL_IDS {
            assert!(spec_for_id(id).is_some(), "missing spec for {id}");
        }
    }

    #[test]
    fn whisper_model_filename_uses_ggml_prefix() {
        assert_eq!(whisper_model_filename("base.en"), "ggml-base.en.bin");
    }
}

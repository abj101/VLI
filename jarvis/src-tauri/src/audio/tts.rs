use std::collections::hash_map::DefaultHasher;
use std::env;
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tauri::{AppHandle, Manager};

const DEFAULT_PIPER_MODEL_FILE: &str = "en_US-amy-medium.onnx";
const CACHE_DIR_NAME: &str = "tts-cache";

#[derive(Debug, Clone)]
struct PiperConfig {
    binary_path: PathBuf,
    model_path: PathBuf,
    cache_dir: PathBuf,
}

pub fn speak_with_piper(app: &AppHandle, text: &str) -> Result<(), String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Speak text cannot be empty".to_string());
    }

    let cfg = resolve_piper_config(app)?;
    std::fs::create_dir_all(&cfg.cache_dir)
        .map_err(|e| format!("failed to create TTS cache dir `{}`: {e}", cfg.cache_dir.display()))?;

    let wav_path = cfg
        .cache_dir
        .join(format!("{}.wav", cache_key(&cfg.model_path, trimmed)));
    if !wav_path.is_file() {
        synthesize_to_wav(&cfg, trimmed, &wav_path)?;
    }

    play_wav_blocking(&wav_path)
}

fn resolve_piper_config(app: &AppHandle) -> Result<PiperConfig, String> {
    let binary_candidates = piper_binary_candidates(app);
    let model_candidates = piper_model_candidates(app);

    let binary_path = first_existing_path(&binary_candidates).ok_or_else(|| {
        format!(
            "Piper runtime not found. Set `JARVIS_PIPER_BIN` (or `PIPER_BIN`) or place `piper.exe` at one of: {}",
            display_candidates(&binary_candidates)
        )
    })?;

    let model_path = first_existing_path(&model_candidates).ok_or_else(|| {
        format!(
            "Piper model not found. Set `JARVIS_PIPER_MODEL` (or `PIPER_MODEL`) or place `{DEFAULT_PIPER_MODEL_FILE}` at one of: {}",
            display_candidates(&model_candidates)
        )
    })?;

    let cache_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir for TTS cache: {e}"))?
        .join(CACHE_DIR_NAME);

    Ok(PiperConfig {
        binary_path,
        model_path,
        cache_dir,
    })
}

fn piper_binary_candidates(app: &AppHandle) -> Vec<PathBuf> {
    let mut out = Vec::new();
    push_env_candidate(&mut out, "JARVIS_PIPER_BIN");
    push_env_candidate(&mut out, "PIPER_BIN");
    if let Ok(resource_dir) = app.path().resource_dir() {
        out.push(resource_dir.join("piper").join("piper.exe"));
    }
    out.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("piper")
            .join("piper.exe"),
    );
    out
}

fn piper_model_candidates(app: &AppHandle) -> Vec<PathBuf> {
    let mut out = Vec::new();
    push_env_candidate(&mut out, "JARVIS_PIPER_MODEL");
    push_env_candidate(&mut out, "PIPER_MODEL");
    if let Ok(resource_dir) = app.path().resource_dir() {
        out.push(
            resource_dir
                .join("piper")
                .join(DEFAULT_PIPER_MODEL_FILE),
        );
    }
    out.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("piper")
            .join(DEFAULT_PIPER_MODEL_FILE),
    );
    out
}

fn push_env_candidate(out: &mut Vec<PathBuf>, var_name: &str) {
    if let Some(path) = env_path(var_name) {
        out.push(path);
    }
}

fn env_path(var_name: &str) -> Option<PathBuf> {
    let value: OsString = env::var_os(var_name)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn first_existing_path(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.is_file()).cloned()
}

fn display_candidates(candidates: &[PathBuf]) -> String {
    candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn cache_key(model_path: &Path, text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    model_path.to_string_lossy().hash(&mut hasher);
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn synthesize_to_wav(cfg: &PiperConfig, text: &str, output_wav: &Path) -> Result<(), String> {
    let mut child = Command::new(&cfg.binary_path)
        .arg("--model")
        .arg(&cfg.model_path)
        .arg("--output_file")
        .arg(output_wav)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "Piper runtime not found at `{}`",
                    cfg.binary_path.display()
                )
            } else {
                format!(
                    "failed to start Piper runtime `{}`: {e}",
                    cfg.binary_path.display()
                )
            }
        })?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .map_err(|e| format!("failed writing text to Piper stdin: {e}"))?;
    } else {
        return Err("failed writing text to Piper stdin: stdin unavailable".to_string());
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed waiting for Piper runtime: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!(
                "Piper synthesis failed with status {}",
                output.status
            ))
        } else {
            Err(format!("Piper synthesis failed: {stderr}"))
        }
    }
}

fn play_wav_blocking(wav_path: &Path) -> Result<(), String> {
    let wav_escaped = escape_powershell_single_quoted(&wav_path.to_string_lossy());
    let command = format!(
        "$p = New-Object System.Media.SoundPlayer '{wav_escaped}'; $p.Load(); $p.PlaySync();"
    );
    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(command)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "PowerShell executable not found while playing Speak audio".to_string()
            } else {
                format!("failed to launch PowerShell for Speak audio playback: {e}")
            }
        })?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!(
                "failed to play Speak audio file `{}` (status {})",
                wav_path.display(),
                output.status
            ))
        } else {
            Err(format!("failed to play Speak audio file `{}`: {stderr}", wav_path.display()))
        }
    }
}

fn escape_powershell_single_quoted(input: &str) -> String {
    input.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::{cache_key, escape_powershell_single_quoted};
    use std::path::Path;

    #[test]
    fn cache_key_is_stable() {
        let model = Path::new("voice.onnx");
        let a = cache_key(model, "hello there");
        let b = cache_key(model, "hello there");
        assert_eq!(a, b);
    }

    #[test]
    fn cache_key_changes_for_different_text() {
        let model = Path::new("voice.onnx");
        let a = cache_key(model, "hello there");
        let b = cache_key(model, "goodbye");
        assert_ne!(a, b);
    }

    #[test]
    fn single_quote_is_escaped_for_powershell() {
        assert_eq!(
            escape_powershell_single_quoted("C:\\tmp\\it's.wav"),
            "C:\\tmp\\it''s.wav"
        );
    }
}

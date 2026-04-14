//! Wake-word detection (`WakeDetector` trait). Porcupine is the default Windows path (feature `porcupine`).

#[cfg(feature = "porcupine")]
pub mod porcupine;

#[cfg(feature = "oww")]
pub mod oww;

use std::path::{Path, PathBuf};
use thiserror::Error;
use tauri::{AppHandle, Manager};

/// ONNX wake classifier filename under `resource_root/oww/` (keep aligned with `oww.rs`).
const OWW_MARKER_ONNX: &str = "hey_jarvis_v0.1.onnx";
/// Bundled Porcupine keyword file under `resource_root/porcupine/`.
const PORCUPINE_MARKER_PPN: &str = "porcupine_windows.ppn";

/// Pick `resource_root` containing `oww/` and/or `porcupine/` trees.
///
/// In `tauri dev`, [`AppHandle::path().resource_dir`] often resolves next to the exe
/// (`target/debug/`) where wake assets are not copied; when markers are missing there,
/// **debug** builds fall back to `src-tauri/resources`. **Release** builds use the Tauri
/// path even if empty so errors refer to the install layout, not the build machine.
fn wake_resource_root_from_candidates(
    tauri_dir: Option<&Path>,
    manifest_resources: &Path,
) -> PathBuf {
    if let Some(dir) = tauri_dir {
        let has_wake_assets = dir.join("oww").join(OWW_MARKER_ONNX).is_file()
            || dir.join("porcupine").join(PORCUPINE_MARKER_PPN).is_file();
        if has_wake_assets {
            return dir.to_path_buf();
        }
    }
    if cfg!(debug_assertions) {
        manifest_resources.to_path_buf()
    } else {
        tauri_dir
            .map(Path::to_path_buf)
            .unwrap_or_else(|| manifest_resources.to_path_buf())
    }
}

/// Root directory passed to [`build_wake_detector`] (`porcupine/` + `oww/` live underneath).
pub(crate) fn resolve_wake_resource_root(app: &AppHandle) -> PathBuf {
    let manifest_res = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    wake_resource_root_from_candidates(
        app.path().resource_dir().ok().as_deref(),
        &manifest_res,
    )
}

/// Picovoice service name for the Porcupine access key (OS keychain).
pub const KEYRING_SERVICE_PORCUPINE: &str = "jarvis-porcupine";
/// Credential entry label for the access key string.
pub const KEYRING_PORCUPINE_ACCESS_KEY: &str = "access_key";

/// Feed PCM chunks (16 kHz, mono, i16). Implementations validate frame length per engine.
pub trait WakeDetector: Send + 'static {
    /// Feed one chunk of PCM (16 kHz, i16, mono). Returns `true` when the wake phrase is detected.
    fn process_frame(&mut self, pcm: &[i16]) -> Result<bool, WakeError>;
    fn backend_name(&self) -> &'static str;
    /// `Some(n)` → caller must pass exactly `n` i16 samples per [`process_frame`] (Porcupine).
    /// `None` → variable-sized chunks (OpenWakeWord accumulates internally).
    fn fixed_input_frame_len(&self) -> Option<usize> {
        None
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WakeError {
    #[error("invalid pcm frame length: expected {expected}, got {actual}")]
    BadFrameLength { expected: usize, actual: usize },
    #[error("wake engine init failed: {0}")]
    Init(String),
    #[error("wake engine process failed: {0}")]
    Process(String),
    #[error("wake engine library: {0}")]
    Library(String),
}

pub(crate) fn expect_pcm_frame_len(actual: usize, expected: usize) -> Result<(), WakeError> {
    if actual != expected {
        Err(WakeError::BadFrameLength { expected, actual })
    } else {
        Ok(())
    }
}

pub mod thread;

/// Build a wake detector for `wake_engine` (`porcupine` / `oww`). Returns `Err` if models/keys missing.
pub(crate) fn build_wake_detector(
    engine: &str,
    resource_dir: &std::path::Path,
    settings: &crate::db::AppSettings,
) -> Result<Box<dyn WakeDetector>, String> {
    match engine {
        "porcupine" => {
            #[cfg(feature = "porcupine")]
            {
                use crate::audio::wake::porcupine::PorcupineBackend;
                PorcupineBackend::try_new(resource_dir)
                    .map(|d| Box::new(d) as Box<dyn WakeDetector>)
                    .map_err(|e| e.to_string())
            }
            #[cfg(not(feature = "porcupine"))]
            {
                let _ = (resource_dir, settings);
                Err("porcupine feature disabled in this build".into())
            }
        }
        "oww" => {
            #[cfg(feature = "oww")]
            {
                try_open_wake_word_oww(resource_dir, settings)
                    .map(|d| Box::new(d) as Box<dyn WakeDetector>)
                    .map_err(|e| e.to_string())
            }
            #[cfg(not(feature = "oww"))]
            {
                let _ = (resource_dir, settings);
                Err("oww feature disabled in this build".into())
            }
        }
        other => Err(format!("unknown wake_engine `{other}`")),
    }
}

/// OpenWakeWord backend using persisted [`crate::db::AppSettings::oww_threshold`].
///
/// Prefer [`build_wake_detector`] from the wake orchestrator so SQLite threshold is always applied.
#[cfg(feature = "oww")]
#[allow(dead_code)]
pub fn try_open_wake_word_oww(
    resource_dir: &std::path::Path,
    settings: &crate::db::AppSettings,
) -> Result<oww::OpenWakeWordBackend, WakeError> {
    oww::OpenWakeWordBackend::try_new(resource_dir, settings.oww_threshold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct MockWakeDetector {
        frame_len: usize,
        remaining_until_fire: usize,
    }

    impl MockWakeDetector {
        fn new(frame_len: usize, fire_after_frames: usize) -> Self {
            Self {
                frame_len,
                remaining_until_fire: fire_after_frames,
            }
        }
    }

    impl WakeDetector for MockWakeDetector {
        fn process_frame(&mut self, pcm: &[i16]) -> Result<bool, WakeError> {
            expect_pcm_frame_len(pcm.len(), self.frame_len)?;
            if self.remaining_until_fire > 0 {
                self.remaining_until_fire -= 1;
                Ok(false)
            } else {
                Ok(true)
            }
        }

        fn backend_name(&self) -> &'static str {
            "mock"
        }

        fn fixed_input_frame_len(&self) -> Option<usize> {
            Some(self.frame_len)
        }
    }

    #[test]
    fn expect_pcm_frame_len_rejects_mismatch() {
        let err = expect_pcm_frame_len(3, 512).unwrap_err();
        assert!(matches!(
            err,
            WakeError::BadFrameLength {
                expected: 512,
                actual: 3
            }
        ));
    }

    #[test]
    fn mock_wake_detector_returns_true_after_n_frames() {
        let mut d = MockWakeDetector::new(4, 1);
        assert_eq!(d.process_frame(&[0i16; 4]), Ok(false));
        assert_eq!(d.process_frame(&[0i16; 4]), Ok(true));
    }

    #[test]
    fn mock_wake_detector_rejects_wrong_length() {
        let mut d = MockWakeDetector::new(4, 0);
        assert!(d.process_frame(&[0i16; 3]).is_err());
    }

    #[test]
    fn wake_resource_root_prefers_tauri_dir_when_oww_marker_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let oww_dir = tmp.path().join("oww");
        std::fs::create_dir_all(&oww_dir).expect("mkdir");
        std::fs::write(oww_dir.join(OWW_MARKER_ONNX), b"x").expect("write");
        let manifest = PathBuf::from("/nonexistent/manifest/resources");
        let picked = wake_resource_root_from_candidates(Some(tmp.path()), &manifest);
        assert_eq!(picked, tmp.path());
    }

    #[test]
    fn wake_resource_root_falls_back_to_manifest_in_debug_when_tauri_dir_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest = tmp.path().join("resources");
        let picked = wake_resource_root_from_candidates(Some(tmp.path()), &manifest);
        if cfg!(debug_assertions) {
            assert_eq!(picked, manifest);
        } else {
            assert_eq!(picked, tmp.path());
        }
    }

    #[test]
    fn build_wake_detector_rejects_unknown_engine() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = crate::db::AppSettings {
            porcupine_key_stored: false,
            wake_engine: "hotkey".into(),
            oww_threshold: 0.5,
            stt_provider: "local".into(),
            remote_stt_url: String::new(),
            remote_stt_model: None,
            remote_stt_timeout_secs: 30,
            remote_stt_key_stored: false,
        };
        let r = build_wake_detector("hotkey", tmp.path(), &s);
        assert!(r.is_err());
    }
}

//! Non-Windows: Porcupine is not wired; `try_new` always fails so the app stays hotkey-only.

use crate::audio::wake::{WakeDetector, WakeError};
use std::path::Path;

/// Placeholder type — construct only via [`Self::try_new`], which returns `Err` off Windows.
pub struct PorcupineBackend;

impl PorcupineBackend {
    pub fn try_new(_resource_dir: &Path) -> Result<Self, WakeError> {
        Err(WakeError::Init(
            "Porcupine wake backend is only supported on Windows in this build".into(),
        ))
    }
}

impl WakeDetector for PorcupineBackend {
    fn process_frame(&mut self, _pcm: &[i16]) -> Result<bool, WakeError> {
        Err(WakeError::Process(
            "Porcupine backend is unavailable on this platform".into(),
        ))
    }

    fn backend_name(&self) -> &'static str {
        "porcupine"
    }
}

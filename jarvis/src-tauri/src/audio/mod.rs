//! Mic capture (cpal) + Whisper STT (Task 4).

pub mod capture;
pub mod stt;

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tauri::AppHandle;

use capture::CaptureSession;
use stt::{spawn_stt_thread, resolve_whisper_model_path};
use whisper_rs::{WhisperContext, WhisperContextParameters};

/// Owns live mic stream + STT worker; dropping stops capture and closes the PCM channel.
pub struct AudioPipeline {
    capture: CaptureSession,
    stt: JoinHandle<()>,
}

impl AudioPipeline {
    /// Starts default input device → PCM `mpsc` → Whisper thread. Emits Tauri events from both.
    pub fn start(app: &AppHandle) -> Result<Self, String> {
        let model_path = resolve_whisper_model_path(app)?;
        let ctx = WhisperContext::new_with_params(
            model_path.to_string_lossy().as_ref(),
            WhisperContextParameters::default(),
        )
        .map_err(|e| format!("failed to load whisper model: {e}"))?;

        let (pcm_tx, pcm_rx) = std::sync::mpsc::channel();
        let (capture, sample_rate) = capture::start_capture(app.clone(), pcm_tx)?;
        let stt = spawn_stt_thread(app.clone(), ctx, pcm_rx, sample_rate);
        Ok(Self { capture, stt })
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        self.capture.stop();
        let _ = self.stt.join();
    }
}

pub type SharedAudioPipeline = Arc<Mutex<Option<AudioPipeline>>>;

/// Clears any live pipeline (stops capture and joins STT).
pub fn stop_shared_pipeline(slot: &SharedAudioPipeline) {
    *slot.lock().unwrap() = None;
}

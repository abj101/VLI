//! Mic capture (cpal) + Whisper STT (Task 4).

pub mod capture;
pub mod stt;

use log::debug;

use std::ops::Deref;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tauri::AppHandle;
use tauri::Emitter;

use capture::CaptureSession;
use stt::{resolve_whisper_model_path, spawn_stt_thread};
use whisper_rs::{WhisperContext, WhisperContextParameters};

/// Consumes PCM when STT is unavailable so the capture thread keeps running and amplitude events fire.
fn spawn_pcm_drain(pcm_rx: Receiver<Vec<f32>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        while pcm_rx.recv().is_ok() {}
    })
}

/// Owns live mic stream + STT worker; dropping stops capture and closes the PCM channel.
pub struct AudioPipeline {
    capture: CaptureSession,
    stt: Option<JoinHandle<()>>,
}

impl AudioPipeline {
    /// Starts default input → PCM channel. Whisper/STT starts only when the model loads; mic + amplitude always run if capture succeeds.
    pub fn start(app: &AppHandle, hud_session_id: u64) -> Result<Self, String> {
        let (pcm_tx, pcm_rx) = std::sync::mpsc::channel();
        let (capture, sample_rate) = capture::start_capture(app.clone(), pcm_tx)?;

        let stt = match resolve_whisper_model_path(app) {
            Ok(model_path) => match WhisperContext::new_with_params(
                model_path.to_string_lossy().as_ref(),
                WhisperContextParameters::default(),
            ) {
                Ok(ctx) => Some(spawn_stt_thread(
                    app.clone(),
                    ctx,
                    pcm_rx,
                    sample_rate,
                    hud_session_id,
                )),
                Err(e) => {
                    let _ = app.emit(
                        "audio-error",
                        serde_json::json!({ "message": format!("failed to load whisper model: {e}") }),
                    );
                    Some(spawn_pcm_drain(pcm_rx))
                }
            },
            Err(msg) => {
                let _ = app.emit("audio-error", serde_json::json!({ "message": msg }));
                Some(spawn_pcm_drain(pcm_rx))
            }
        };

        Ok(Self { capture, stt })
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        self.capture.stop();
        if let Some(h) = self.stt.take() {
            // Do not join: Whisper's final decode in `stt_loop` can take seconds and would block
            // whatever thread runs this Drop (often the Tauri event path). Detach and let the STT
            // thread finish in the background.
            std::mem::forget(h);
        }
    }
}

/// Shared mic/STT pipeline handle stored in Tauri state.
///
/// # Safety
/// `cpal::Stream` is not `Send`, but this app only starts/stops capture from the main thread
/// and Tauri UI commands that touch this state are serialized on the runtime used here.
#[derive(Clone)]
pub struct SharedAudioPipeline(pub Arc<Mutex<Option<AudioPipeline>>>);

unsafe impl Send for SharedAudioPipeline {}
unsafe impl Sync for SharedAudioPipeline {}

impl Deref for SharedAudioPipeline {
    type Target = Arc<Mutex<Option<AudioPipeline>>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Clears any live pipeline (stops capture and joins STT).
///
/// **Must not** drop [`AudioPipeline`] while holding `slot`'s mutex: `Drop` joins the STT thread,
/// which may emit `transcript-update` and re-enter code that tries to lock the same mutex → deadlock.
/// `AudioPipeline` is not `Send` (cpal), so we cannot move it to another thread; we `take()` then
/// `drop` on this thread after releasing the mutex.
pub fn stop_shared_pipeline(slot: &SharedAudioPipeline) {
    debug!("audio: stop_shared_pipeline (take pipeline, drop outside mutex)");
    let old = {
        let mut g = slot.lock().unwrap();
        g.take()
    };
    drop(old);
    debug!("audio: stop_shared_pipeline complete");
}

#[cfg(test)]
mod tests {
    use super::spawn_pcm_drain;

    #[test]
    fn pcm_drain_joins_after_sender_dropped() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(vec![0.5f32]).unwrap();
        drop(tx);
        spawn_pcm_drain(rx).join().expect("drain thread should exit");
    }
}

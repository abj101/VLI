//! Mic capture (cpal) + STT pipeline (Whisper local / OS stub / remote HTTP) (Phase 4).

pub mod capture;
pub mod preroll;
pub mod stt;
pub mod transcript_event;
pub mod transcription;
pub mod tts;
pub mod wake;
pub mod whisper_models;

use log::debug;

use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tauri::AppHandle;
use tauri::Emitter;
use tauri::Manager;

use capture::CaptureSession;
use stt::{load_whisper_context_serialized, spawn_stt_thread};
use transcription::{spawn_os_stt_thread, spawn_remote_stt_thread};

pub use preroll::WakePreroll;
pub use transcription::RemoteSttParams;

/// When true, the wake thread releases the default mic so the listen pipeline can open it.
#[derive(Clone)]
pub struct WakeMicSuppressed(pub Arc<AtomicBool>);

/// Resolved STT path for [`AudioPipeline::start`].
pub enum SttPipelineChoice {
    /// Local Whisper; `use_gpu` is passed to whisper.cpp when supported by the build.
    Local { use_gpu: bool },
    Os,
    Remote(RemoteSttParams),
}

/// Consumes PCM when STT is unavailable so the capture thread keeps running and amplitude events fire.
fn spawn_pcm_drain(pcm_rx: Receiver<Vec<f32>>) -> JoinHandle<()> {
    std::thread::spawn(move || while pcm_rx.recv().is_ok() {})
}

/// Owns live mic stream + STT worker; dropping stops capture and closes the PCM channel.
pub struct AudioPipeline {
    capture: CaptureSession,
    stt: Option<JoinHandle<()>>,
    /// Signals STT workers to skip further Whisper decode after stop/drop.
    cancel: Arc<AtomicBool>,
}

impl AudioPipeline {
    /// Tell the STT worker to skip further Whisper decode. Capture stops on drop.
    pub fn signal_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl AudioPipeline {
    /// Starts default input → PCM channel. STT worker depends on `choice`; mic + amplitude run if capture succeeds.
    /// `wake_preroll_16k` — recent wake-mic audio at 16 kHz (mono f32) to seed Whisper after wake word.
    pub fn start(
        app: &AppHandle,
        hud_session_id: u64,
        choice: SttPipelineChoice,
        wake_preroll_16k: Vec<f32>,
        for_dictation: bool,
    ) -> Result<Self, String> {
        let (pcm_tx, pcm_rx) = std::sync::mpsc::channel();
        let (capture, sample_rate) = capture::start_capture(app.clone(), pcm_tx, true)?;
        let cancel = Arc::new(AtomicBool::new(false));

        let stt = match choice {
            SttPipelineChoice::Os => Some(spawn_os_stt_thread(app.clone(), pcm_rx, hud_session_id)),
            SttPipelineChoice::Remote(params) => {
                if params.endpoint.trim().is_empty() {
                    let _ = app.emit(
                        "audio-error",
                        serde_json::json!({ "message": "Remote STT: set an HTTPS endpoint URL in Settings." }),
                    );
                    Some(spawn_pcm_drain(pcm_rx))
                } else {
                    Some(spawn_remote_stt_thread(
                        app.clone(),
                        params,
                        pcm_rx,
                        sample_rate,
                        hud_session_id,
                        for_dictation,
                    ))
                }
            }
            SttPipelineChoice::Local { use_gpu } => {
                let model_id = crate::open_db_connection(app)
                    .ok()
                    .and_then(|conn| crate::db::get_app_settings(&conn).ok())
                    .map(|s| s.local_whisper_model)
                    .unwrap_or_else(|| stt::DEFAULT_LOCAL_WHISPER_MODEL.to_string());
                let app_load = app.clone();
                let app_emit = app.clone();
                let cancel_loader = Arc::clone(&cancel);
                let cancel_stt = Arc::clone(&cancel);
                let warmed = app
                    .try_state::<crate::WhisperModelCache>()
                    .and_then(|c| c.take_context());
                Some(crate::gpu_startup::spawn_whisper_loader_thread("whisper-loader", move || {
                    if cancel_loader.load(Ordering::Relaxed) {
                        std::mem::forget(spawn_pcm_drain(pcm_rx));
                        return;
                    }
                    let model_path = match whisper_models::ensure_whisper_model(
                        &app_emit,
                        model_id.as_str(),
                    ) {
                        Ok(path) => path,
                        Err(msg) => {
                            let _ = app_emit.emit(
                                "audio-error",
                                serde_json::json!({ "message": msg }),
                            );
                            std::mem::forget(spawn_pcm_drain(pcm_rx));
                            return;
                        }
                    };
                    if cancel_loader.load(Ordering::Relaxed) {
                        std::mem::forget(spawn_pcm_drain(pcm_rx));
                        return;
                    }
                    let model_path_text = model_path.to_string_lossy().to_string();
                    let load_result = warmed
                        .map(|ctx| Ok((ctx, use_gpu)))
                        .unwrap_or_else(|| {
                            load_whisper_context_serialized(model_path_text.as_str(), use_gpu)
                        });
                    match load_result {
                        Ok((ctx, effective_gpu)) => {
                            if cancel_loader.load(Ordering::Relaxed) {
                                std::mem::forget(spawn_pcm_drain(pcm_rx));
                                return;
                            }
                            if use_gpu && !effective_gpu {
                                let _ = app_emit.emit(
                                    "audio-error",
                                    serde_json::json!({ "message": "Whisper GPU init failed; using CPU instead." }),
                                );
                            }
                            let inner = spawn_stt_thread(
                                app_load,
                                ctx,
                                pcm_rx,
                                sample_rate,
                                hud_session_id,
                                effective_gpu,
                                wake_preroll_16k,
                                for_dictation,
                                cancel_stt,
                            );
                            std::mem::forget(inner);
                        }
                        Err(msg) => {
                            let _ = app_emit.emit(
                                "audio-error",
                                serde_json::json!({ "message": msg }),
                            );
                            std::mem::forget(spawn_pcm_drain(pcm_rx));
                        }
                    }
                }).map_err(|e| e.to_string())?)
            }
        };

        Ok(Self {
            capture,
            stt,
            cancel,
        })
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.capture.stop();
        if let Some(h) = self.stt.take() {
            // Do not join: detach the worker; `cancel` tells it to skip further Whisper decode.
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
pub fn stop_shared_pipeline(app: &AppHandle, slot: &SharedAudioPipeline) {
    if let Some(suppressed) = app.try_state::<WakeMicSuppressed>() {
        suppressed.0.store(false, Ordering::SeqCst);
    }
    debug!("audio: stop_shared_pipeline (take pipeline, drop outside mutex)");
    let old = {
        let mut g = slot.lock().unwrap();
        if let Some(pipeline) = g.as_ref() {
            pipeline.signal_cancel();
        }
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
        spawn_pcm_drain(rx)
            .join()
            .expect("drain thread should exit");
    }
}

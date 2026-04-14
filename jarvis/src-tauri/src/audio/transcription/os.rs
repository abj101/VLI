//! OS-native speech recognition path (stub until platform STT is integrated).

use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use tauri::{AppHandle, Emitter};

/// Starts a worker that keeps capture alive and reports that OS STT is not wired yet.
pub fn spawn_os_stt_thread(
    app: AppHandle,
    pcm_rx: Receiver<Vec<f32>>,
    hud_session_id: u64,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let msg = if cfg!(target_os = "windows") {
            "OS speech-to-text is not yet integrated (stub; use Local or Remote STT)."
        } else {
            "OS speech-to-text is not available on this platform yet."
        };
        let _ = app.emit(
            "audio-error",
            serde_json::json!({ "message": msg, "hudSessionId": hud_session_id }),
        );
        while pcm_rx.recv().is_ok() {}
    })
}

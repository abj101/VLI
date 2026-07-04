//! macOS helpers (microphone TCC, overlay window presentation).

#[cfg(target_os = "macos")]
pub mod panel;
#[cfg(target_os = "macos")]
pub mod window;

#[cfg(target_os = "macos")]
pub use panel::{init_hud_panel, reassert_hud_overlay};

use log::{info, warn};
use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use std::sync::mpsc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Shown in HUD when mic access is denied or restricted.
pub const MIC_DENIED_MESSAGE: &str = "Microphone access is denied. Open System Settings → Privacy & Security → Microphone, enable jarvis, then restart. If jarvis is not listed, run in Terminal: tccutil reset Microphone com.jarvis.app";

const MIC_UNDETERMINED_MESSAGE: &str =
    "Microphone permission was not granted. Voice features will not work until you allow microphone access.";

const MIC_TIMEOUT_MESSAGE: &str =
    "Microphone permission prompt timed out. Restart jarvis and allow microphone access when prompted.";

fn audio_media_type() -> &'static objc2_av_foundation::AVMediaType {
    unsafe { AVMediaTypeAudio.expect("AVMediaTypeAudio constant") }
}

fn microphone_authorization_status() -> AVAuthorizationStatus {
    unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio_media_type()) }
}

fn authorization_status_label(status: AVAuthorizationStatus) -> &'static str {
    if status == AVAuthorizationStatus::Authorized {
        "authorized"
    } else if status == AVAuthorizationStatus::NotDetermined {
        "not_determined"
    } else if status == AVAuthorizationStatus::Denied {
        "denied"
    } else if status == AVAuthorizationStatus::Restricted {
        "restricted"
    } else {
        "unknown"
    }
}

fn log_microphone_status(context: &str) {
    let status = microphone_authorization_status();
    info!(
        "macos microphone authorization ({context}): {}",
        authorization_status_label(status)
    );
}

fn activate_app_for_permission_prompt() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let ns_app = NSApplication::sharedApplication(mtm);
    ns_app.activateIgnoringOtherApps(true);
}

/// True when the user must fix access in System Settings (no prompt will appear).
pub fn microphone_permission_is_denied() -> bool {
    let status = microphone_authorization_status();
    status == AVAuthorizationStatus::Denied || status == AVAuthorizationStatus::Restricted
}

fn fire_microphone_access_request(tx: mpsc::SyncSender<bool>) {
    unsafe {
        let handler = block2::RcBlock::new(move |granted: objc2::runtime::Bool| {
            let _ = tx.send(granted.as_bool());
        });
        AVCaptureDevice::requestAccessForMediaType_completionHandler(
            audio_media_type(),
            &*handler,
        );
    }
}

/// Prompt on the main thread; blocks the caller until the user responds (up to 30s).
fn request_microphone_access_blocking(app: &AppHandle) -> Option<bool> {
    let (tx, rx) = mpsc::sync_channel(1);

    let dispatch_request = |tx: mpsc::SyncSender<bool>| {
        activate_app_for_permission_prompt();
        fire_microphone_access_request(tx);
    };

    if MainThreadMarker::new().is_some() {
        dispatch_request(tx);
    } else if app
        .run_on_main_thread(move || dispatch_request(tx))
        .is_err()
    {
        warn!("macos mic permission request could not run on main thread");
        return None;
    }

    match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(granted) => Some(granted),
        Err(_) => {
            warn!("macos mic permission request timed out");
            None
        }
    }
}

fn resolve_microphone_access(app: &AppHandle) -> Result<(), &'static str> {
    let status = microphone_authorization_status();
    match status {
        AVAuthorizationStatus::Authorized => Ok(()),
        AVAuthorizationStatus::NotDetermined => match request_microphone_access_blocking(app) {
            Some(true) => Ok(()),
            Some(false) => Err(MIC_DENIED_MESSAGE),
            None => Err(MIC_TIMEOUT_MESSAGE),
        },
        AVAuthorizationStatus::Denied | AVAuthorizationStatus::Restricted => Err(MIC_DENIED_MESSAGE),
        _ => {
            warn!("macos mic permission returned unexpected status");
            Err(MIC_UNDETERMINED_MESSAGE)
        }
    }
}

/// Run after the event loop is up so the system permission sheet can present.
pub fn schedule_microphone_permission_request(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        let app_for_main = app.clone();
        if app
            .run_on_main_thread(move || ensure_microphone_permission(&app_for_main))
            .is_err()
        {
            warn!("macos: could not schedule microphone permission on main thread");
        }
    });
}

/// Prompt for mic access when still undetermined; emit a HUD-visible error when denied.
pub fn ensure_microphone_permission(app: &AppHandle) {
    log_microphone_status("ensure");
    if let Err(message) = resolve_microphone_access(app) {
        warn!("microphone permission not granted: {message}");
        let _ = app.emit(
            "audio-error",
            serde_json::json!({ "message": message }),
        );
    }
}

/// Gate cpal capture: returns an error string suitable for HUD display when mic is unavailable.
pub fn require_microphone_for_capture(app: &AppHandle) -> Result<(), String> {
    resolve_microphone_access(app).map_err(String::from)
}

/// Open System Settings → Privacy & Security → Microphone.
pub fn open_microphone_privacy_settings() -> Result<(), String> {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        .status()
        .map_err(|e| format!("failed to open Microphone privacy settings: {e}"))?
        .success()
        .then_some(())
        .ok_or_else(|| "failed to open Microphone privacy settings".to_string())
}

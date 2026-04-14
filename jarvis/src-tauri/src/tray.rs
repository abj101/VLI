//! System tray: pause / resume / quit (Task 8).

use crate::audio::{stop_shared_pipeline, SharedAudioPipeline};
use crate::hud::HudPhase;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::image::Image;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::menu::{Menu, MenuItem};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::tray::TrayIconBuilder;
use tauri::AppHandle;

/// `Listening` shows the HUD capture UI; mic/STT only start when not paused.
pub fn mic_start_allowed(is_paused: &AtomicBool, phase: HudPhase) -> bool {
    phase == HudPhase::Listening && !is_paused.load(Ordering::Relaxed)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn setup_tray(
    app: &AppHandle,
    is_paused: Arc<AtomicBool>,
    audio: SharedAudioPipeline,
) -> tauri::Result<()> {
    let pause_item = MenuItem::with_id(app, "pause-toggle", "Pause", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&pause_item, &quit_item])?;

    let pause_item_for_menu = pause_item.clone();
    let is_paused_for_menu = Arc::clone(&is_paused);
    let audio_for_menu = audio.clone();

    // Embedded PNG: `default_window_icon()` can render blank in the Windows notification area.
    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))
        .expect("embedded tray icon must decode");

    TrayIconBuilder::with_id("main")
        .tooltip("JARVIS")
        .icon(icon)
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "quit" => {
                app.exit(0);
            }
            "pause-toggle" => {
                let old = is_paused_for_menu.load(Ordering::SeqCst);
                let now_paused = !old;
                is_paused_for_menu.store(now_paused, Ordering::SeqCst);
                if now_paused {
                    stop_shared_pipeline(&audio_for_menu);
                }
                let label = if now_paused { "Resume" } else { "Pause" };
                let _ = pause_item_for_menu.set_text(label);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::mic_start_allowed;
    use crate::hud::HudPhase;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn mic_allowed_when_listening_and_not_paused() {
        let p = AtomicBool::new(false);
        assert!(mic_start_allowed(&p, HudPhase::Listening));
    }

    #[test]
    fn mic_blocked_when_paused_even_if_listening() {
        let p = AtomicBool::new(true);
        assert!(!mic_start_allowed(&p, HudPhase::Listening));
    }

    #[test]
    fn mic_not_started_for_non_listening_phases() {
        let p = AtomicBool::new(false);
        assert!(!mic_start_allowed(&p, HudPhase::Idle));
        assert!(!mic_start_allowed(&p, HudPhase::Stopped));
    }
}

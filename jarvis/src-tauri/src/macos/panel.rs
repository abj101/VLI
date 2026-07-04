//! HUD as NSPanel — required for overlay above native fullscreen Spaces (plain NSWindow cannot).

use log::warn;
use std::sync::Arc;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_nspanel::{
    tauri_panel, CollectionBehavior, ManagerExt, Panel, StyleMask, WebviewWindowExt,
};

use crate::hud::HUD_WINDOW_LABEL;

tauri_panel! {
    panel!(HudOverlayPanel {
        config: {
            can_become_key_window: true,
            is_floating_panel: true,
        }
    })
}

/// `NSMainMenuWindowLevel + 1` — above fullscreen app content with `FullScreenAuxiliary`.
const HUD_PANEL_LEVEL: i64 = 25;

fn hud_panel_handle(app: &AppHandle) -> Result<Arc<dyn Panel>, String> {
    app.get_webview_panel(HUD_WINDOW_LABEL)
        .map_err(|_| format!("hud panel not initialized (`{HUD_WINDOW_LABEL}`)"))
}

/// One-time: subclass the Tauri HUD `NSWindow` into `NSPanel` (see tauri-nspanel / lume#203).
pub fn init_hud_panel(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))?;

    if window.set_always_on_top(false).is_err() {
        warn!("hud panel init: could not clear always_on_top (tao may reset level later)");
    }

    let panel = window
        .to_panel::<HudOverlayPanel>()
        .map_err(|e| format!("hud to_panel: {e}"))?;

    apply_overlay_config(panel.as_ref());
    Ok(())
}

/// Collection behavior + level for fullscreen Spaces. Re-run after `show()` (tao can downgrade).
pub fn apply_overlay_config(panel: &dyn Panel) {
    panel.set_level(HUD_PANEL_LEVEL);
    panel.set_floating_panel(true);
    panel.set_style_mask(
        StyleMask::empty()
            .borderless()
            .nonactivating_panel()
            .into(),
    );
    panel.set_collection_behavior(
        CollectionBehavior::new()
            .can_join_all_spaces()
            .stationary()
            .full_screen_auxiliary()
            .ignores_cycle()
            .into(),
    );
    panel.set_hides_on_deactivate(false);
    panel.set_movable_by_window_background(true);
}

pub fn reassert_hud_overlay(app: &AppHandle) {
    if !crate::is_hud_overlay_visible(app) {
        return;
    }
    let Ok(panel) = hud_panel_handle(app) else {
        return;
    };
    apply_overlay_config(panel.as_ref());
    if panel.is_visible() {
        panel.order_front_regardless();
    }
}

pub fn ensure_hud_panel(app: &AppHandle, _window: &WebviewWindow) -> Result<Arc<dyn Panel>, String> {
    match hud_panel_handle(app) {
        Ok(panel) => Ok(panel),
        Err(_) => {
            init_hud_panel(app)?;
            hud_panel_handle(app)
        }
    }
}

//! Present overlay HUD above other apps (incl. fullscreen / zoomed windows).

use log::warn;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSEvent, NSScreen, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{ActivationPolicy, AppHandle, Manager, WebviewWindow};

use crate::macos::panel::{apply_overlay_config, ensure_hud_panel, reassert_hud_overlay};

const HUD_WINDOW_LABEL: &str = "hud";

/// Where to anchor the HUD inside the active screen's visible frame (menu bar / dock excluded).
#[derive(Debug, Clone, Copy)]
pub enum HudScreenPlacement {
    Center,
    Bottom { margin_logical: f64 },
}

fn point_in_rect(point: NSPoint, rect: NSRect) -> bool {
    point.x >= rect.origin.x
        && point.x < rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y < rect.origin.y + rect.size.height
}

fn screen_at_mouse(mtm: MainThreadMarker) -> Option<Retained<NSScreen>> {
    let mouse = NSEvent::mouseLocation();
    for screen in NSScreen::screens(mtm).iter() {
        if point_in_rect(mouse, screen.frame()) {
            return Some(screen.clone());
        }
    }
    NSScreen::mainScreen(mtm)
}

fn placement_screen(
    mtm: MainThreadMarker,
    placement: HudScreenPlacement,
) -> Option<Retained<NSScreen>> {
    match placement {
        HudScreenPlacement::Center => NSScreen::mainScreen(mtm),
        HudScreenPlacement::Bottom { .. } => {
            screen_at_mouse(mtm).or_else(|| NSScreen::mainScreen(mtm))
        }
    }
}

/// Place using AppKit coordinates on the active screen.
pub fn place_hud_on_screen(
    app: &AppHandle,
    placement: HudScreenPlacement,
    window_size: (f64, f64),
    position_guard: Option<&AtomicBool>,
) -> Result<(), String> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("place_hud_on_screen must run on the main thread".into());
    };
    let panel = ensure_hud_panel(app, &hud_window(app)?)?;
    let ns_window = panel.as_panel();
    place_hud_on_screen_guarded(ns_window, mtm, placement, window_size, position_guard);
    Ok(())
}

fn hud_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))
}

fn place_hud_on_screen_guarded(
    ns_window: &NSWindow,
    mtm: MainThreadMarker,
    placement: HudScreenPlacement,
    window_size: (f64, f64),
    position_guard: Option<&AtomicBool>,
) {
    if let Some(guard) = position_guard {
        guard.store(true, Ordering::SeqCst);
    }

    let Some(screen) = placement_screen(mtm, placement) else {
        ns_window.center();
        if let Some(guard) = position_guard {
            guard.store(false, Ordering::SeqCst);
        }
        return;
    };

    let visible = screen.visibleFrame();
    let (win_w, win_h) = window_size;
    let x = visible.origin.x + (visible.size.width - win_w) / 2.0;
    let y = match placement {
        HudScreenPlacement::Center => {
            visible.origin.y + (visible.size.height - win_h) / 2.0
        }
        HudScreenPlacement::Bottom { margin_logical } => visible.origin.y + margin_logical,
    };
    // Apply size + origin together — `frame()` can still reflect the prior command overlay size
    // immediately after Tauri `set_size`, which skews horizontal centering for dictation.
    ns_window.setFrame_display(
        NSRect::new(NSPoint::new(x, y), NSSize::new(win_w, win_h)),
        false,
    );

    if let Some(guard) = position_guard {
        guard.store(false, Ordering::SeqCst);
    }
}

fn schedule_overlay_reassert(app: &AppHandle) {
    let app_now = app.clone();
    let _ = app.run_on_main_thread(move || {
        if crate::is_hud_overlay_visible(&app_now) {
            reassert_hud_overlay(&app_now);
        }
    });

    let app_later = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        if !crate::is_hud_overlay_visible(&app_later) {
            return;
        }
        let app_for_main = app_later.clone();
        let _ = app_later.run_on_main_thread(move || reassert_hud_overlay(&app_for_main));
    });
}

/// Hide NSPanel + webview and return to menu-bar accessory mode (no dock bounce).
pub fn hide_hud_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(HUD_WINDOW_LABEL) {
        if let Ok(panel) = ensure_hud_panel(app, &window) {
            panel.hide();
        }
        if let Err(e) = window.hide() {
            warn!("hud hide: {e}");
        }
    }
    if let Err(e) = app.set_activation_policy(ActivationPolicy::Accessory) {
        warn!("macos hide_hud_overlay: set_activation_policy: {e}");
    }
}

/// Raise the HUD above other apps and make it key. Must run on the main thread.
pub fn present_overlay_window(
    app: &AppHandle,
    window: &WebviewWindow,
    placement: HudScreenPlacement,
    window_size: (f64, f64),
    use_native_placement: bool,
    position_guard: Option<&AtomicBool>,
) -> Result<(), String> {
    if let Err(e) = app.set_activation_policy(ActivationPolicy::Regular) {
        warn!("macos present_overlay: set_activation_policy: {e}");
    }

    let Some(mtm) = MainThreadMarker::new() else {
        return Err("present_overlay_window must run on the main thread".into());
    };

    let ns_app = NSApplication::sharedApplication(mtm);
    ns_app.activateIgnoringOtherApps(true);

    let _ = window.unminimize();
    let _ = window.set_always_on_top(false);

    let panel = ensure_hud_panel(app, window)?;
    apply_overlay_config(panel.as_ref());

    panel.order_front_regardless();
    panel.make_key_and_order_front();

    // Keep Tauri visibility state in sync; panel is the same underlying Cocoa window.
    window.show().map_err(|e| format!("hud show: {e}"))?;

    apply_overlay_config(panel.as_ref());
    panel.order_front_regardless();

    if use_native_placement {
        place_hud_on_screen_guarded(
            panel.as_panel(),
            mtm,
            placement,
            window_size,
            position_guard,
        );
    }

    window.set_focus().map_err(|e| format!("hud set_focus: {e}"))?;
    apply_overlay_config(panel.as_ref());
    panel.order_front_regardless();
    schedule_overlay_reassert(app);
    Ok(())
}

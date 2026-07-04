//! HUD window phase + click-through policy (Task 3).

use crate::db::{self, settings};
use log::warn;
use rusqlite::Connection;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::window::Color;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewWindow};

/// Suppresses persisting HUD position while Rust applies default / restored placement.
#[derive(Debug, Default)]
pub struct HudPositionGuard(pub AtomicBool);

pub const HUD_WINDOW_LABEL: &str = "hud";

/// Command overlay (center screen, transcript + waveform). Dictation overlay (bottom center, waveform only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HudOverlayMode {
    #[default]
    Command,
    Dictation,
}

impl HudOverlayMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Dictation => "dictation",
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "command" => Ok(Self::Command),
            "dictation" => Ok(Self::Dictation),
            other => Err(format!("unknown hud overlay mode `{other}`")),
        }
    }
}

impl Serialize for HudOverlayMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HudOverlayMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ModeVisitor;

        impl Visitor<'_> for ModeVisitor {
            type Value = HudOverlayMode;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a hud overlay mode string (`command` or `dictation`)")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                HudOverlayMode::parse(v).map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_str(ModeVisitor)
    }
}

pub const HUD_COMMAND_WIDTH: f64 = 480.0;
pub const HUD_COMMAND_HEIGHT: f64 = 172.0;
pub const HUD_DICTATION_WIDTH: f64 = 200.0;
pub const HUD_DICTATION_HEIGHT: f64 = 88.0;
/// Logical px gap between dictation HUD bottom edge and monitor work area bottom.
pub const HUD_DICTATION_BOTTOM_MARGIN: f64 = 48.0;

pub fn overlay_logical_size(mode: HudOverlayMode) -> LogicalSize<f64> {
    match mode {
        HudOverlayMode::Command => LogicalSize::new(HUD_COMMAND_WIDTH, HUD_COMMAND_HEIGHT),
        HudOverlayMode::Dictation => LogicalSize::new(HUD_DICTATION_WIDTH, HUD_DICTATION_HEIGHT),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HudLogicalPosition {
    pub x: f64,
    pub y: f64,
}

fn setting_key_for_mode(mode: HudOverlayMode) -> &'static str {
    match mode {
        HudOverlayMode::Command => settings::SETTING_HUD_COMMAND_POSITION,
        HudOverlayMode::Dictation => settings::SETTING_HUD_DICTATION_POSITION,
    }
}

pub fn parse_hud_position(raw: &str) -> Option<HudLogicalPosition> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed: HudLogicalPosition = serde_json::from_str(raw).ok()?;
    if !parsed.x.is_finite() || !parsed.y.is_finite() {
        return None;
    }
    Some(parsed)
}

pub fn load_hud_position(conn: &Connection, mode: HudOverlayMode) -> Option<HudLogicalPosition> {
    settings::get_setting(conn, setting_key_for_mode(mode))
        .ok()
        .flatten()
        .and_then(|raw| parse_hud_position(&raw))
}

pub fn save_hud_position(
    conn: &Connection,
    mode: HudOverlayMode,
    position: HudLogicalPosition,
) -> Result<(), String> {
    let json = serde_json::to_string(&position).map_err(|e| e.to_string())?;
    db::set_setting(conn, setting_key_for_mode(mode), &json).map_err(|e| e.to_string())
}

pub fn hud_logical_position(window: &WebviewWindow) -> Result<HudLogicalPosition, String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    Ok(HudLogicalPosition {
        x: pos.x as f64 / scale,
        y: pos.y as f64 / scale,
    })
}

fn set_hud_position_guarded(
    window: &WebviewWindow,
    guard: &HudPositionGuard,
    position: HudLogicalPosition,
) -> Result<(), String> {
    guard.0.store(true, Ordering::SeqCst);
    let result = window
        .set_position(LogicalPosition::new(position.x, position.y))
        .map_err(|e| e.to_string());
    guard.0.store(false, Ordering::SeqCst);
    result
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum HudPlacementAnchor {
    Center,
    Bottom { margin_logical: f64 },
}

fn hud_placement_monitor(window: &WebviewWindow, app: &AppHandle) -> Result<tauri::Monitor, String> {
    if let Ok(cursor) = app.cursor_position() {
        if let Ok(Some(monitor)) = window.monitor_from_point(cursor.x, cursor.y) {
            return Ok(monitor);
        }
    }
    if let Some(monitor) = window.current_monitor().map_err(|e| e.to_string())? {
        return Ok(monitor);
    }
    window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no monitor available for hud placement".to_string())
}

fn has_saved_hud_position(app: &AppHandle, mode: HudOverlayMode) -> bool {
    open_db_connection(app)
        .ok()
        .and_then(|conn| load_hud_position(&conn, mode))
        .is_some()
}

/// Place within monitor work area (menu bar, dock, taskbar excluded) — matches Tauri `window.center()`.
fn logical_position_in_work_area(
    origin_x: f64,
    origin_y: f64,
    area_w: f64,
    area_h: f64,
    win_w: f64,
    win_h: f64,
    anchor: HudPlacementAnchor,
) -> HudLogicalPosition {
    let x = origin_x + (area_w - win_w) / 2.0;
    let y = match anchor {
        HudPlacementAnchor::Center => origin_y + (area_h - win_h) / 2.0,
        HudPlacementAnchor::Bottom { margin_logical } => {
            origin_y + area_h - win_h - margin_logical
        }
    };
    HudLogicalPosition { x, y }
}

fn position_hud_in_work_area(
    monitor: &tauri::Monitor,
    win_w: f64,
    win_h: f64,
    anchor: HudPlacementAnchor,
) -> HudLogicalPosition {
    let scale = monitor.scale_factor();
    let work = monitor.work_area();
    let area_w = work.size.width as f64 / scale;
    let area_h = work.size.height as f64 / scale;
    let origin_x = work.position.x as f64 / scale;
    let origin_y = work.position.y as f64 / scale;
    logical_position_in_work_area(origin_x, origin_y, area_w, area_h, win_w, win_h, anchor)
}

fn default_hud_position(
    window: &WebviewWindow,
    app: &AppHandle,
    mode: HudOverlayMode,
) -> Result<HudLogicalPosition, String> {
    let monitor = hud_placement_monitor(window, app)?;
    let size = overlay_logical_size(mode);
    let anchor = match mode {
        HudOverlayMode::Command => HudPlacementAnchor::Center,
        HudOverlayMode::Dictation => HudPlacementAnchor::Bottom {
            margin_logical: HUD_DICTATION_BOTTOM_MARGIN,
        },
    };
    Ok(position_hud_in_work_area(
        &monitor,
        size.width,
        size.height,
        anchor,
    ))
}

fn resolve_hud_position(
    app: &AppHandle,
    window: &WebviewWindow,
    mode: HudOverlayMode,
) -> Result<HudLogicalPosition, String> {
    let saved = open_db_connection(app)
        .ok()
        .and_then(|conn| load_hud_position(&conn, mode));
    match saved {
        Some(pos) => Ok(pos),
        None => default_hud_position(window, app, mode),
    }
}

fn apply_hud_window_position(
    app: &AppHandle,
    window: &WebviewWindow,
    mode: HudOverlayMode,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if !has_saved_hud_position(app, mode) {
        // Default placement uses native AppKit coordinates in `present_overlay_window`.
        return Ok(());
    }

    let guard = app.state::<HudPositionGuard>();
    let position = resolve_hud_position(app, window, mode)?;
    set_hud_position_guarded(window, &guard, position)
}

fn macos_screen_placement(mode: HudOverlayMode) -> crate::macos::window::HudScreenPlacement {
    match mode {
        HudOverlayMode::Command => crate::macos::window::HudScreenPlacement::Center,
        HudOverlayMode::Dictation => crate::macos::window::HudScreenPlacement::Bottom {
            margin_logical: HUD_DICTATION_BOTTOM_MARGIN,
        },
    }
}

pub fn apply_hud_window_geometry(
    app: &AppHandle,
    mode: HudOverlayMode,
) -> Result<(), String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))?;

    let size = overlay_logical_size(mode);
    window
        .set_size(size)
        .map_err(|e| format!("hud set_size: {e}"))?;

    apply_hud_window_position(app, &window, mode)
}

pub fn persist_hud_window_position(app: &AppHandle, mode: HudOverlayMode) -> Result<(), String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))?;
    let position = hud_logical_position(&window)?;
    let conn = open_db_connection(app)?;
    save_hud_position(&conn, mode, position)
}

fn open_db_connection(app: &AppHandle) -> Result<Connection, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Connection::open(dir.join("jarvis.db")).map_err(|e| e.to_string())
}

/// Mirrors `HudPhase` in `jarvis/src/types.ts` (snake_case strings on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HudPhase {
    #[default]
    Idle,
    Listening,
    Matched,
    Routing,
    Executing,
    AwaitingInput,
    Done,
    Stopped,
}

impl HudPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Listening => "listening",
            Self::Matched => "matched",
            Self::Routing => "routing",
            Self::Executing => "executing",
            Self::AwaitingInput => "awaiting_input",
            Self::Done => "done",
            Self::Stopped => "stopped",
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "idle" => Ok(Self::Idle),
            "listening" => Ok(Self::Listening),
            "matched" => Ok(Self::Matched),
            "routing" => Ok(Self::Routing),
            "executing" => Ok(Self::Executing),
            "awaiting_input" => Ok(Self::AwaitingInput),
            "done" => Ok(Self::Done),
            "stopped" => Ok(Self::Stopped),
            other => Err(format!("unknown hud phase `{other}`")),
        }
    }
}

impl Serialize for HudPhase {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HudPhase {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PhaseVisitor;

        impl Visitor<'_> for PhaseVisitor {
            type Value = HudPhase;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a snake_case hud phase string")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                HudPhase::parse(v).map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_str(PhaseVisitor)
    }
}

/// `true` → window ignores mouse (clicks pass through to desktop).
pub fn ignore_cursor_for_phase(phase: HudPhase) -> bool {
    matches!(
        phase,
        HudPhase::Idle
            | HudPhase::Matched
            | HudPhase::Routing
            | HudPhase::Executing
            | HudPhase::Done
            | HudPhase::Stopped
    )
}

pub fn sync_hud_window(app: &AppHandle, phase: HudPhase) -> Result<(), String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))?;
    window
        .set_ignore_cursor_events(ignore_cursor_for_phase(phase))
        .map_err(|e| e.to_string())
}

/// Fully transparent webview background (matches `transparent` + `backgroundColor` in `tauri.conf.json`).
/// On Windows, WebView2 treats non-zero alpha as opaque — resetting avoids a dark rim during fades.
pub fn sync_hud_webview_background(app: &AppHandle) {
    let Some(w) = app.get_webview_window(HUD_WINDOW_LABEL) else {
        return;
    };
    if let Err(e) = w.set_background_color(Some(Color(0, 0, 0, 0))) {
        warn!("hud webview transparent background: {e}");
    }
}

/// Like [`present_hud_window`] but hops to the main thread when required (macOS AppKit).
pub fn present_hud_window_on_main(app: &AppHandle, mode: HudOverlayMode) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if objc2::MainThreadMarker::new().is_none() {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let app_for_main = app.clone();
            app.run_on_main_thread(move || {
                let result = present_hud_window_resolved(&app_for_main, mode);
                let _ = tx.send(result);
            })
            .map_err(|e| format!("hud present main-thread dispatch: {e:?}"))?;
            return rx
                .recv()
                .map_err(|e| format!("hud present main-thread result: {e}"))?;
        }
    }
    present_hud_window_resolved(app, mode)
}

/// Hide the HUD native window. macOS NSPanel / activation policy must run on the main thread.
pub fn hide_hud_window_on_main(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        if objc2::MainThreadMarker::new().is_none() {
            let app_for_main = app.clone();
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            if let Err(e) = app.run_on_main_thread(move || {
                crate::macos::window::hide_hud_overlay(&app_for_main);
                let _ = tx.send(());
            }) {
                warn!("hud hide main-thread dispatch: {e:?}");
                return;
            }
            if let Err(e) = rx.recv() {
                warn!("hud hide main-thread result: {e}");
            }
            return;
        }
        crate::macos::window::hide_hud_overlay(app);
        return;
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(w) = app.get_webview_window(HUD_WINDOW_LABEL) {
        if let Err(e) = w.hide() {
            warn!("hud hide: {e}");
        }
    }
}

fn present_hud_window_resolved(app: &AppHandle, mode: HudOverlayMode) -> Result<(), String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))?;
    present_hud_window(app, &window, mode)
}

/// Show/focus the HUD native window. On macOS tray/dev builds, `show()` alone can land behind other apps.
pub fn present_hud_window(
    app: &AppHandle,
    window: &WebviewWindow,
    mode: HudOverlayMode,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let size = overlay_logical_size(mode);
        let window_size = (size.width, size.height);
        let use_native_placement = !has_saved_hud_position(app, mode);
        let guard = app.state::<HudPositionGuard>();
        let guard_ref = if use_native_placement {
            Some(&guard.0)
        } else {
            None
        };
        crate::macos::window::present_overlay_window(
            app,
            window,
            macos_screen_placement(mode),
            window_size,
            use_native_placement,
            guard_ref,
        )?;
        if use_native_placement {
            let placement = macos_screen_placement(mode);
            let app_defer = app.clone();
            let _ = app.run_on_main_thread(move || {
                let guard = app_defer.state::<HudPositionGuard>();
                let _ = crate::macos::window::place_hud_on_screen(
                    &app_defer,
                    placement,
                    window_size,
                    Some(&guard.0),
                );
                crate::macos::reassert_hud_overlay(&app_defer);
            });
        }
        if !use_native_placement {
            apply_hud_window_position(app, window, mode)?;
        }
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ignore_cursor_for_phase, parse_hud_position, HudLogicalPosition, HudOverlayMode,
        HudPhase, overlay_logical_size,
    };

    #[test]
    fn hud_position_json_round_trip() {
        let pos = HudLogicalPosition { x: 120.5, y: 840.0 };
        let json = serde_json::to_string(&pos).unwrap();
        assert_eq!(parse_hud_position(&json), Some(pos));
    }

    #[test]
    fn hud_position_rejects_non_finite() {
        assert!(parse_hud_position(r#"{"x":null,"y":1}"#).is_none());
    }

    #[test]
    fn overlay_mode_round_trip_strings() {
        for mode in [HudOverlayMode::Command, HudOverlayMode::Dictation] {
            assert_eq!(HudOverlayMode::parse(mode.as_str()).unwrap(), mode);
        }
    }

    #[test]
    fn dictation_overlay_is_compact() {
        let size = overlay_logical_size(HudOverlayMode::Dictation);
        assert!(size.width < super::HUD_COMMAND_WIDTH);
        assert!(size.height < super::HUD_COMMAND_HEIGHT);
    }

    #[test]
    fn work_area_center_offsets_below_menu_bar() {
        let pos = super::logical_position_in_work_area(
            0.0,
            25.0,
            1440.0,
            875.0,
            480.0,
            172.0,
            super::HudPlacementAnchor::Center,
        );
        assert_eq!(pos.x, 480.0);
        assert_eq!(pos.y, 376.5);
    }

    #[test]
    fn work_area_bottom_center_respects_margin() {
        let pos = super::logical_position_in_work_area(
            0.0,
            25.0,
            1440.0,
            875.0,
            200.0,
            88.0,
            super::HudPlacementAnchor::Bottom {
                margin_logical: 48.0,
            },
        );
        assert_eq!(pos.x, 620.0);
        assert_eq!(pos.y, 764.0);
    }

    #[test]
    fn click_through_when_passive_phases() {
        assert!(ignore_cursor_for_phase(HudPhase::Idle));
        assert!(ignore_cursor_for_phase(HudPhase::Matched));
        assert!(ignore_cursor_for_phase(HudPhase::Routing));
        assert!(ignore_cursor_for_phase(HudPhase::Executing));
        assert!(ignore_cursor_for_phase(HudPhase::Done));
        assert!(ignore_cursor_for_phase(HudPhase::Stopped));
    }

    #[test]
    fn interactive_when_listening_or_awaiting_follow_up() {
        assert!(!ignore_cursor_for_phase(HudPhase::Listening));
        assert!(!ignore_cursor_for_phase(HudPhase::AwaitingInput));
    }

    #[test]
    fn phase_round_trip_strings() {
        for p in [
            HudPhase::Idle,
            HudPhase::Listening,
            HudPhase::Matched,
            HudPhase::Routing,
            HudPhase::Executing,
            HudPhase::AwaitingInput,
            HudPhase::Done,
            HudPhase::Stopped,
        ] {
            assert_eq!(HudPhase::parse(p.as_str()).unwrap(), p);
        }
    }
}

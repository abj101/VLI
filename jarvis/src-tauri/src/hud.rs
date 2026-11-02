//! HUD window phase + click-through policy (Task 3).

use crate::db::{self, settings};
use log::warn;
use rusqlite::Connection;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Listener, LogicalPosition, LogicalSize, Manager, WebviewWindow};

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
pub const HUD_DICTATION_WIDTH: f64 = 24.0;
pub const HUD_DICTATION_HEIGHT: f64 = 24.0;
// Must match the transparent 2px margin on .hud-root--dictation in App.css.
const DICTATION_EDGE_INSET: f64 = 2.0;
/// Logical px gap between dictation HUD bottom edge and monitor work area bottom.
pub const HUD_DICTATION_BOTTOM_MARGIN: f64 = 48.0;

pub fn overlay_logical_size(mode: HudOverlayMode) -> LogicalSize<f64> {
    match mode {
        HudOverlayMode::Command => LogicalSize::new(HUD_COMMAND_WIDTH, HUD_COMMAND_HEIGHT),
        HudOverlayMode::Dictation => LogicalSize::new(
            HUD_DICTATION_WIDTH + DICTATION_EDGE_INSET * 2.0,
            HUD_DICTATION_HEIGHT + DICTATION_EDGE_INSET * 2.0,
        ),
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

/// Physical-px rect `(left, top, right, bottom)`.
pub type PxRect = (i32, i32, i32, i32);

/// Top-left for a `(w, h)` HUD beside `t`: right, left, below, above - first that fits `work`;
/// else the right-hand spot clamped on-screen. Left/right centre on `t`, below/above align to its left.
pub fn anchor_beside(t: PxRect, (w, h): (i32, i32), work: PxRect, gap: i32) -> (i32, i32) {
    let mid_y = (t.1 + t.3 - h) / 2;
    let sides = [
        (t.2 + gap, mid_y),
        (t.0 - gap - w, mid_y),
        (t.0, t.3 + gap),
        (t.0, t.1 - gap - h),
    ];
    let fits =
        |&(x, y): &(i32, i32)| x >= work.0 && y >= work.1 && x + w <= work.2 && y + h <= work.3;
    let (x, y) = sides.iter().copied().find(fits).unwrap_or(sides[0]);
    (x.min(work.2 - w).max(work.0), y.min(work.3 - h).max(work.1))
}

static DICTATION_TYPING: AtomicBool = AtomicBool::new(false);

/// Set while dictated text is being typed, so the orb tracks the caret every frame.
pub fn set_dictation_typing(on: bool) {
    DICTATION_TYPING.store(on, Ordering::Relaxed);
}

/// Orb diameter for an anchor rect: the caret's line height (so it sits within the line and
/// matches the font), or the default beside a selection / text box. All physical px.
pub fn orb_diameter(rect: PxRect, scale: f64) -> i32 {
    let px = |l: f64| (l * scale).round() as i32;
    if rect.2 - rect.0 <= 3 {
        (rect.3 - rect.1).clamp(px(16.0), px(32.0))
    } else {
        px(HUD_DICTATION_WIDTH)
    }
}

const HUD_MORPH_SECONDS: f64 = 0.380;

/// Zero velocity and acceleration at both ends keep the first few frames from jumping.
fn morph_rect(from: [f64; 4], to: [f64; 4], elapsed: f64) -> [f64; 4] {
    let t = (elapsed / HUD_MORPH_SECONDS).clamp(0.0, 1.0);
    let eased = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    std::array::from_fn(|i| from[i] + (to[i] - from[i]) * eased)
}

/// Keeps the dictation orb beside the text caret (or selection / text box) until the session ends.
/// With `morph`, glide and shrink the same visible surface after its shape is ready.
/// ponytail: polls every frame while typing, else every ~50 ms; SetWinEventHook
/// (EVENT_OBJECT_LOCATIONCHANGE on OBJID_CARET) if CPU shows up.
#[cfg(windows)]
pub fn follow_text_box(app: AppHandle, session_id: u64, morph: bool) {
    use crate::window::focused_text_rect;
    use windows::core::BOOL;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SWP_NOACTIVATE,
        SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };
    std::thread::spawn(move || {
        let Some(window) = app.get_webview_window(HUD_WINDOW_LABEL) else {
            return;
        };
        let Ok(hwnd) = window.hwnd() else { return };
        let (Ok(start), Ok(size)) = (window.outer_position(), window.outer_size()) else {
            return;
        };
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let ready_listener = app.listen("hud-dictation-ready", move |event| {
            if serde_json::from_str::<u64>(event.payload()).ok() == Some(session_id) {
                let _ = ready_tx.try_send(());
            }
        });
        let scale = window.scale_factor().unwrap_or(1.0);
        // (x, y, w, h) in physical px.
        let mut cur = [
            start.x as f64,
            start.y as f64,
            size.width as f64,
            size.height as f64,
        ];
        let mut placed = [start.x, start.y, size.width as i32, size.height as i32];
        let mut target = None::<[f64; 4]>;
        let mut animations = BOOL(1);
        let _ = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some(std::ptr::from_mut(&mut animations).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        let animate = animations.as_bool();
        let mut origin = cur;
        let mut needs_morph = morph;
        let mut morphing = morph && animate;
        let mut morph_started = None::<std::time::Instant>;
        let mut first_placement = true;
        let mut frame = 0u32;
        let mut last = std::time::Instant::now();
        while crate::dictation::accepts(&app, session_id) {
            let typing = DICTATION_TYPING.load(Ordering::Relaxed);
            if target.is_none() || typing || frame % 3 == 0 {
                let destination = focused_text_rect()
                    .map(|(rect, work)| {
                        let target_scale = window
                            .monitor_from_point(rect.0 as f64, rect.1 as f64)
                            .ok()
                            .flatten()
                            .map(|m| m.scale_factor())
                            .unwrap_or(scale);
                        let d = orb_diameter(rect, target_scale)
                            + (DICTATION_EDGE_INSET * 2.0 * target_scale).round() as i32;
                        let gap = (6.0 * target_scale).round() as i32;
                        let (x, y) = anchor_beside(rect, (d, d), work, gap);
                        [x as f64, y as f64, d as f64, d as f64]
                    });
                if destination.is_none() {
                    if let Err(err) = crate::dictation::set_input_available(&app, session_id, false) {
                        warn!("dictation input pause: {err}");
                        break;
                    }
                    if crate::dictation::waiting_expired(&app, session_id) {
                        let _ = crate::dictation::stop_dictation(
                            &app, &app.state::<crate::SharedHud>(), &app.state::<crate::audio::SharedAudioPipeline>(),
                        );
                        break;
                    }
                    if let (Ok(position), Ok(size)) = (window.outer_position(), window.outer_size()) {
                        origin = [position.x as f64, position.y as f64, size.width as f64, size.height as f64];
                        cur = origin;
                        placed = [position.x, position.y, size.width as i32, size.height as i32];
                    }
                    target = None;
                    needs_morph = true;
                    morphing = animate;
                    first_placement = true;
                    last = std::time::Instant::now();
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    continue;
                }
                if crate::dictation::waiting_for_input(&app) {
                    // Readiness belongs to this morph, not the previous caret placement.
                    while ready_rx.try_recv().is_ok() {}
                }
                if let Err(err) = crate::dictation::set_input_available(&app, session_id, true) {
                    warn!("dictation input resume: {err}");
                    break;
                }
                if let Some(t) = destination {
                    if target.is_none() {
                        if needs_morph {
                            // Commit the original outline before the first native resize.
                            while crate::dictation::accepts(&app, session_id)
                                && ready_rx
                                    .recv_timeout(std::time::Duration::from_millis(50))
                                    .is_err()
                            {}
                        }
                        if !morphing {
                            cur = t;
                        }
                        morph_started = Some(std::time::Instant::now());
                    }
                    target = Some(t);
                }
            }
            frame += 1;
            let dt = last.elapsed().as_secs_f64();
            last = std::time::Instant::now();
            if let Some(t) = target {
                if morphing {
                    let elapsed = morph_started.unwrap().elapsed().as_secs_f64();
                    cur = morph_rect(origin, t, elapsed);
                    morphing = elapsed < HUD_MORPH_SECONDS;
                } else {
                    let rate = if typing { 45.0 } else { 18.0 };
                    let k = if animate {
                        1.0 - (-rate * dt).exp()
                    } else {
                        1.0
                    };
                    for i in 0..4 {
                        cur[i] += (t[i] - cur[i]) * k;
                    }
                }
                let next = cur.map(|v| v.round() as i32);
                // A single native update avoids rendering a resized frame at the previous position.
                if next != placed {
                    // A caret query can outlive cancellation or a replacement session.
                    if !crate::dictation::accepts(&app, session_id) {
                        break;
                    }
                    if unsafe {
                        SetWindowPos(
                            HWND(hwnd.0),
                            None,
                            next[0],
                            next[1],
                            next[2],
                            next[3],
                            SWP_NOACTIVATE | SWP_NOZORDER,
                        )
                    }
                    .is_err()
                    {
                        break;
                    }
                    placed = next;
                }
                if !morphing
                    && (first_placement || ready_rx.try_recv().is_ok())
                    && crate::dictation::accepts(&app, session_id)
                {
                    let _ = app.emit("hud-dictation-placed", session_id);
                    first_placement = false;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        app.unlisten(ready_listener);
    });
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum HudPlacementAnchor {
    Center,
    Bottom { margin_logical: f64 },
}

fn hud_placement_monitor(
    window: &WebviewWindow,
    app: &AppHandle,
) -> Result<tauri::Monitor, String> {
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

#[cfg(target_os = "macos")]
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
        HudPlacementAnchor::Bottom { margin_logical } => origin_y + area_h - win_h - margin_logical,
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

#[cfg(target_os = "macos")]
fn macos_screen_placement(mode: HudOverlayMode) -> crate::macos::window::HudScreenPlacement {
    match mode {
        HudOverlayMode::Command => crate::macos::window::HudScreenPlacement::Center,
        HudOverlayMode::Dictation => crate::macos::window::HudScreenPlacement::Bottom {
            margin_logical: HUD_DICTATION_BOTTOM_MARGIN,
        },
    }
}

pub fn apply_hud_window_geometry(app: &AppHandle, mode: HudOverlayMode) -> Result<(), String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))?;

    let size = overlay_logical_size(mode);
    window
        .set_size(size)
        .map_err(|e| format!("hud set_size: {e}"))?;

    apply_hud_window_position(app, &window, mode)
}

/// Waiting dictation uses the main HUD footprint at the center without taking text focus.
pub fn present_dictation_waiting(app: &AppHandle) -> Result<(), String> {
    let window = app.get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "missing HUD window".to_string())?;
    window.set_size(overlay_logical_size(HudOverlayMode::Command)).map_err(|e| e.to_string())?;
    let position = default_hud_position(&window, app, HudOverlayMode::Command)?;
    set_hud_position_guarded(&window, &app.state::<HudPositionGuard>(), position)?;
    present_hud_window(app, &window, HudOverlayMode::Command)?;
    sync_hud_webview_background(app);
    Ok(())
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
    // Dictation HUD sits beside the user's text: never steal clicks or drags there.
    window
        .set_ignore_cursor_events(
            ignore_cursor_for_phase(phase) || crate::dictation::is_active(app),
        )
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

/// Show the HUD native window (focused on macOS, where `show()` alone can land behind other apps).
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
        let _ = (app, mode);
        // Never take focus: shortcuts and typed text must land in the app the user was in.
        window.set_focusable(false).map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn morph_has_a_gentle_first_frame_and_finishes_without_overshooting() {
        let from = [800.0, 400.0, 480.0, 172.0];
        let to = [120.0, 900.0, 28.0, 28.0];
        assert_eq!(super::morph_rect(from, to, 0.0), from);
        let first = super::morph_rect(from, to, 1.0 / 60.0);
        assert!((first[2] - from[2]).abs() < 1.0);
        for fps in [30, 60, 144] {
            let frames = (super::HUD_MORPH_SECONDS * fps as f64).ceil() as usize;
            let mut previous = from;
            for frame in 1..=frames {
                let next = super::morph_rect(from, to, frame as f64 / fps as f64);
                for i in 0..4 {
                    assert!((next[i] - to[i]).abs() <= (previous[i] - to[i]).abs() + 1e-9);
                    assert!(next[i] >= from[i].min(to[i]) && next[i] <= from[i].max(to[i]));
                }
                previous = next;
            }
            assert_eq!(previous, to);
        }
    }

    #[test]
    fn orb_matches_caret_line_height_within_limits() {
        assert_eq!(orb_diameter((10, 0, 11, 20), 1.0), 20);
        assert_eq!(orb_diameter((10, 0, 11, 20), 2.0), 32); // 20 px caret at 2x is below the 16 px floor
        assert_eq!(orb_diameter((10, 0, 11, 90), 1.0), 32);
        assert_eq!(orb_diameter((10, 0, 300, 40), 1.0), 24); // text box: default size
    }

    #[test]
    fn anchor_beside_picks_first_side_that_fits() {
        let work = (0, 0, 1000, 800);
        let hud = (100, 50);
        // right
        assert_eq!(
            anchor_beside((100, 300, 300, 340), hud, work, 10),
            (310, 295)
        );
        // left (no room right)
        assert_eq!(
            anchor_beside((800, 300, 950, 340), hud, work, 10),
            (690, 295)
        );
        // below (target spans width)
        assert_eq!(anchor_beside((0, 100, 1000, 140), hud, work, 10), (0, 150));
        // above (target spans width, no room below)
        assert_eq!(anchor_beside((0, 740, 1000, 790), hud, work, 10), (0, 680));
        // fits nowhere: clamped on-screen
        let (x, y) = anchor_beside((0, 0, 1000, 800), hud, work, 10);
        assert!((0..=900).contains(&x) && (0..=750).contains(&y));
    }

    use super::{
        anchor_beside, ignore_cursor_for_phase, orb_diameter, overlay_logical_size,
        parse_hud_position, HudLogicalPosition, HudOverlayMode, HudPhase,
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
        assert_eq!(size.width, super::HUD_DICTATION_WIDTH + 4.0);
        assert_eq!(size.height, super::HUD_DICTATION_HEIGHT + 4.0);
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

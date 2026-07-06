//! macOS window zone placement via System Events (Accessibility).

use crate::process::hidden_command;
use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
use std::collections::HashSet;
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const LAUNCH_PLACE_TIMEOUT: Duration = Duration::from_secs(3);

pub const DEFAULT_MONITOR: &str = "monitor_primary";

pub type WindowSnapshot = HashSet<String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ZoneRect {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

pub fn validate_placement_zone(zone: &str) -> Result<&'static str, String> {
    match zone.trim() {
        "left_half" => Ok("left_half"),
        "right_half" => Ok("right_half"),
        "maximize" => Ok("maximize"),
        other => Err(format!("unsupported placement zone `{other}`")),
    }
}

pub fn placement_status_label(zone: &str) -> String {
    match zone {
        "left_half" => "left".into(),
        "right_half" => "right".into(),
        "maximize" => "maximized".into(),
        other => other.to_string(),
    }
}

pub fn snapshot_top_level_windows() -> WindowSnapshot {
    match run_osascript_output(snapshot_windows_script()) {
        Ok(output) => output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
        Err(_) => HashSet::new(),
    }
}

pub fn place_window_after_app_launch(
    exe_path: &str,
    zone: &str,
    monitor: &str,
    before: &WindowSnapshot,
) -> Result<(), String> {
    let zone = validate_placement_zone(zone)?;
    let target = app_match_token(exe_path);
    let deadline = Instant::now() + LAUNCH_PLACE_TIMEOUT;
    while Instant::now() < deadline {
        let current = snapshot_top_level_windows();
        for id in current.difference(before) {
            if window_id_matches_app(id, &target) {
                return snap_window_by_id(id, zone, monitor);
            }
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(format!(
        "timed out waiting for a new `{target}` window to place ({zone})"
    ))
}

pub fn focus_existing_app_window(
    exe_path: &str,
    display_name: &str,
    zone: Option<&str>,
    monitor: &str,
) -> Result<bool, String> {
    let token = if !display_name.trim().is_empty() {
        display_name.to_string()
    } else {
        app_match_token(exe_path)
    };
    let snapshot = snapshot_top_level_windows();
    let needle = token.to_ascii_lowercase();
    let Some(id) = snapshot.into_iter().find(|id| {
        let lower = id.to_ascii_lowercase();
        lower.starts_with(&format!("{needle}|")) || lower.contains(&format!("|{needle}|"))
    }) else {
        return Ok(false);
    };
    if let Some(zone) = zone.map(str::trim).filter(|z| !z.is_empty()) {
        snap_window_by_id(&id, validate_placement_zone(zone)?, monitor)?;
    } else {
        focus_window_by_id(&id)?;
    }
    Ok(true)
}

pub fn snap_foreground_window(zone: &str, monitor: &str) -> Result<(), String> {
    let zone = validate_placement_zone(zone)?;
    let rect = zone_rect(zone, monitor)?;
    run_osascript(snap_foreground_script(rect))
}

fn snap_window_by_id(id: &str, zone: &str, monitor: &str) -> Result<(), String> {
    let rect = zone_rect(zone, monitor)?;
    let (process, window_name) = parse_window_id(id)?;
    run_osascript(snap_named_window_script(&process, &window_name, rect))
}

fn focus_window_by_id(id: &str) -> Result<(), String> {
    let (process, window_name) = parse_window_id(id)?;
    run_osascript(focus_named_window_script(&process, &window_name))
}

fn parse_window_id(id: &str) -> Result<(String, String), String> {
    let mut parts = id.splitn(2, '|');
    let process = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("invalid window id `{id}`"))?
        .to_string();
    let window_name = parts.next().unwrap_or("").to_string();
    Ok((process, window_name))
}

fn app_match_token(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.ends_with(".app") {
        return trimmed
            .trim_end_matches(".app")
            .rsplit('/')
            .next()
            .unwrap_or(trimmed)
            .to_string();
    }
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

fn window_id_matches_app(id: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let lower_id = id.to_ascii_lowercase();
    let lower_token = token.to_ascii_lowercase();
    lower_id.starts_with(&format!("{lower_token}|"))
        || lower_id.contains(&format!("|{lower_token}|"))
}

/// Compute Cocoa screen coordinates for a placement zone.
pub fn zone_rect(zone: &str, monitor: &str) -> Result<ZoneRect, String> {
    let _ = monitor;
    let zone = validate_placement_zone(zone)?;
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("window placement must run on the main thread".into());
    };
    let Some(screen) = NSScreen::mainScreen(mtm) else {
        return Err("no main screen available for window placement".into());
    };
    let visible = screen.visibleFrame();
    zone_rect_from_visible_frame(
        zone,
        visible.origin.x.round() as i64,
        visible.origin.y.round() as i64,
        visible.size.width.round() as i64,
        visible.size.height.round() as i64,
    )
}

pub(crate) fn zone_rect_from_visible_frame(
    zone: &str,
    x0: i64,
    y0: i64,
    width: i64,
    height: i64,
) -> Result<ZoneRect, String> {
    let zone = validate_placement_zone(zone)?;
    Ok(match zone {
        "maximize" => ZoneRect {
            x: x0,
            y: y0,
            width,
            height,
        },
        "left_half" => ZoneRect {
            x: x0,
            y: y0,
            width: width / 2,
            height,
        },
        "right_half" => ZoneRect {
            x: x0 + width / 2,
            y: y0,
            width: width / 2,
            height,
        },
        _ => return Err(format!("unsupported placement zone `{zone}`")),
    })
}

fn run_osascript(script: String) -> Result<(), String> {
    let output = hidden_command("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| format!("osascript failed: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "window placement failed: {stderr} (grant Accessibility permission in System Settings)"
        ))
    }
}

fn run_osascript_output(script: String) -> Result<String, String> {
    let output = hidden_command("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| format!("osascript failed: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "window snapshot failed: {stderr} (grant Accessibility permission in System Settings)"
        ))
    }
}

fn snapshot_windows_script() -> String {
    r#"tell application "System Events"
  set out to ""
  repeat with p in (every application process whose visible is true)
    set pName to name of p
    repeat with w in (every window of p)
      try
        set wName to name of w as text
        set out to out & pName & "|" & wName & linefeed
      end try
    end repeat
  end repeat
  return out
end tell"#
    .to_string()
}

fn snap_foreground_script(rect: ZoneRect) -> String {
    format!(
        r#"tell application "System Events"
  set frontProc to first application process whose frontmost is true
  tell frontProc
    set position of window 1 to {{{x}, {y}}}
    set size of window 1 to {{{w}, {h}}}
  end tell
end tell"#,
        x = rect.x,
        y = rect.y,
        w = rect.width,
        h = rect.height,
    )
}

fn snap_named_window_script(process: &str, window_name: &str, rect: ZoneRect) -> String {
    let process = applescript_escape(process);
    let window_name = applescript_escape(window_name);
    format!(
        r#"tell application "System Events"
  tell application process "{process}"
    repeat with w in (every window whose name is "{window_name}")
      set position of w to {{{x}, {y}}}
      set size of w to {{{w}, {h}}}
    end repeat
  end tell
end tell"#,
        process = process,
        window_name = window_name,
        x = rect.x,
        y = rect.y,
        w = rect.width,
        h = rect.height,
    )
}

fn focus_named_window_script(process: &str, window_name: &str) -> String {
    let process = applescript_escape(process);
    let window_name = applescript_escape(window_name);
    format!(
        r#"tell application "System Events"
  tell application process "{process}"
    set frontmost to true
    repeat with w in (every window whose name is "{window_name}")
      perform action "AXRaise" of w
    end repeat
  end tell
end tell"#,
        process = process,
        window_name = window_name,
    )
}

fn applescript_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::{validate_placement_zone, zone_rect_from_visible_frame};

    #[test]
    fn validates_zones() {
        assert_eq!(validate_placement_zone("left_half").unwrap(), "left_half");
        assert!(validate_placement_zone("invalid").is_err());
    }

    #[test]
    fn zone_rect_left_half_width_is_half_of_screen() {
        let rect = zone_rect_from_visible_frame("left_half", 0, 0, 1920, 1080).expect("rect");
        let full = zone_rect_from_visible_frame("maximize", 0, 0, 1920, 1080).expect("full");
        assert_eq!(rect.height, full.height);
        assert_eq!(rect.width, full.width / 2);
        assert_eq!(rect.x, full.x);
    }

    #[test]
    fn zone_rect_right_half_starts_midscreen() {
        let left = zone_rect_from_visible_frame("left_half", 0, 0, 1920, 1080).expect("left");
        let right = zone_rect_from_visible_frame("right_half", 0, 0, 1920, 1080).expect("right");
        assert_eq!(right.x, left.x + left.width);
        assert_eq!(right.width, left.width);
    }
}

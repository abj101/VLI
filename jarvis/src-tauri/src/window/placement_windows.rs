//! Win32 window zone placement: half-screen snap, maximize, foreground snap.

use rapidfuzz::fuzz;
use std::collections::HashSet;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITOR_DEFAULTTOPRIMARY,
    MONITOR_DEFAULTTONEAREST, MONITORINFO,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetAncestor, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible, SetForegroundWindow, SetWindowPos, ShowWindow, GA_ROOT, HWND_TOP,
    SW_MAXIMIZE, SW_RESTORE, SWP_NOZORDER, SWP_SHOWWINDOW,
};

/// Minimum title / display-name fuzzy ratio for existing-window focus.
const TITLE_MATCH_MIN_RATIO: f64 = 0.72;

pub const DEFAULT_MONITOR: &str = "monitor_primary";

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const LAUNCH_PLACE_TIMEOUT: Duration = Duration::from_secs(3);

pub type WindowSnapshot = HashSet<isize>;

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
    let mut hwnds = WindowSnapshot::new();
    unsafe {
        let _ = EnumWindows(Some(enum_collect_visible), LPARAM(&mut hwnds as *mut _ as isize));
    }
    hwnds
}

pub fn place_window_after_app_launch(
    exe_path: &str,
    zone: &str,
    monitor: &str,
    before: &WindowSnapshot,
) -> Result<(), String> {
    let zone = validate_placement_zone(zone)?;
    let target_exe = exe_basename(exe_path);
    let deadline = Instant::now() + LAUNCH_PLACE_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(hwnd) = find_new_window_for_exe(&target_exe, before) {
            return apply_zone(hwnd, zone, monitor);
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(format!(
        "timed out waiting for a new `{target_exe}` window to place ({zone})"
    ))
}

/// Focus an already-running app window when possible; optionally snap to `zone`.
/// Returns `true` when an existing window was focused (no new launch needed).
pub fn focus_existing_app_window(
    exe_path: &str,
    display_name: &str,
    zone: Option<&str>,
    monitor: &str,
) -> Result<bool, String> {
    let Some(hwnd) = find_existing_window_for_app(exe_path, display_name) else {
        return Ok(false);
    };
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
    if let Some(zone) = zone.map(str::trim).filter(|z| !z.is_empty()) {
        apply_zone(hwnd, validate_placement_zone(zone)?, monitor)?;
    }
    Ok(true)
}

pub fn snap_foreground_window(zone: &str, monitor: &str) -> Result<(), String> {
    let zone = validate_placement_zone(zone)?;
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return Err("no foreground window to snap".to_string());
    }
    apply_zone(hwnd, zone, monitor)
}

fn apply_zone(hwnd: HWND, zone: &str, monitor: &str) -> Result<(), String> {
    match zone {
        "maximize" => unsafe {
            let _ = ShowWindow(hwnd, SW_MAXIMIZE);
        },
        "left_half" | "right_half" => {
            let work = monitor_work_area(hwnd, monitor)?;
            let width = (work.right - work.left) / 2;
            let height = work.bottom - work.top;
            let (x, w) = if zone == "left_half" {
                (work.left, width)
            } else {
                (work.left + width, width)
            };
            unsafe {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                SetWindowPos(
                    hwnd,
                    Some(HWND_TOP),
                    x,
                    work.top,
                    w,
                    height,
                    SWP_NOZORDER | SWP_SHOWWINDOW,
                )
                .map_err(|e| format!("SetWindowPos failed: {e}"))?;
            }
        }
        _ => return Err(format!("unsupported placement zone `{zone}`")),
    }
    Ok(())
}

fn monitor_work_area(hwnd: HWND, monitor: &str) -> Result<RECT, String> {
    let hmon = match monitor.trim() {
        DEFAULT_MONITOR | "" => unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) },
        other => {
            return Err(format!("unsupported monitor selector `{other}` (use `{DEFAULT_MONITOR}`)"));
        }
    };
    let hmon = if hmon.0.is_null() {
        unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) }
    } else {
        hmon
    };
    if hmon.0.is_null() {
        return Err("could not resolve monitor for placement".to_string());
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetMonitorInfoW(hmon, &mut info)
            .ok()
            .map_err(|e| format!("GetMonitorInfoW failed: {e}"))?;
    }
    Ok(info.rcWork)
}

fn find_existing_window_for_app(exe_path: &str, display_name: &str) -> Option<HWND> {
    let target_exe = exe_basename(exe_path);
    let display_lower = display_name.trim().to_lowercase();
    let mut best: Option<(f64, HWND)> = None;
    let ctx = ExistingFindCtx {
        target_exe,
        display_lower: &display_lower,
        best: &mut best,
    };
    unsafe {
        let _ = EnumWindows(Some(enum_find_existing), LPARAM(&ctx as *const _ as isize));
    }
    best.map(|(_, hwnd)| hwnd)
}

struct ExistingFindCtx<'a> {
    target_exe: String,
    display_lower: &'a str,
    best: &'a mut Option<(f64, HWND)>,
}

unsafe extern "system" fn enum_find_existing(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut ExistingFindCtx<'_>);
    if !is_placeable_top_level(hwnd) {
        return TRUE;
    }
    let mut score = 0.0f64;
    if let Some(pid) = window_pid(hwnd) {
        if let Some(image) = process_image_name(pid) {
            if exe_names_match(&ctx.target_exe, &image) {
                score = score.max(1.0);
            }
        }
    }
    if !ctx.display_lower.is_empty() {
        if let Some(title) = window_title(hwnd) {
            let title_lower = title.to_lowercase();
            let ratio = title_matches_display(ctx.display_lower, &title_lower);
            if ratio >= TITLE_MATCH_MIN_RATIO {
                score = score.max(ratio);
            }
        }
    }
    if score > 0.0 {
        let replace = match ctx.best {
            None => true,
            Some((prev, _)) => score > *prev + 1e-9,
        };
        if replace {
            *ctx.best = Some((score, hwnd));
        }
    }
    TRUE
}

fn title_matches_display(display_lower: &str, title_lower: &str) -> f64 {
    if display_lower.is_empty() || title_lower.is_empty() {
        return 0.0;
    }
    if title_lower == display_lower {
        return 1.0;
    }
    if title_lower.starts_with(display_lower) {
        let boundary = title_lower.chars().nth(display_lower.len());
        if boundary.map(|c| !c.is_ascii_alphanumeric()).unwrap_or(true) {
            return 0.95;
        }
    }
    fuzz::ratio(display_lower.chars(), title_lower.chars())
}

fn window_title(hwnd: HWND) -> Option<String> {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

fn find_new_window_for_exe(target_exe: &str, before: &WindowSnapshot) -> Option<HWND> {
    let mut found = None;
    let ctx = FindCtx {
        target_exe: target_exe.to_string(),
        before,
        found: &mut found,
    };
    unsafe {
        let _ = EnumWindows(Some(enum_find_new), LPARAM(&ctx as *const _ as isize));
    }
    found
}

struct FindCtx<'a> {
    target_exe: String,
    before: &'a WindowSnapshot,
    found: &'a mut Option<HWND>,
}

unsafe extern "system" fn enum_collect_visible(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let set = &mut *(lparam.0 as *mut WindowSnapshot);
    if is_placeable_top_level(hwnd) {
        set.insert(hwnd.0 as isize);
    }
    TRUE
}

unsafe extern "system" fn enum_find_new(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut FindCtx<'_>);
    if ctx.found.is_some() {
        return TRUE;
    }
    if !is_placeable_top_level(hwnd) {
        return TRUE;
    }
    if ctx.before.contains(&(hwnd.0 as isize)) {
        return TRUE;
    }
    let Some(pid) = window_pid(hwnd) else {
        return TRUE;
    };
    if let Some(image) = process_image_name(pid) {
        if exe_names_match(&ctx.target_exe, &image) {
            *ctx.found = Some(hwnd);
        }
    }
    TRUE
}

fn is_placeable_top_level(hwnd: HWND) -> bool {
    unsafe {
        if hwnd.0.is_null() || !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        GetAncestor(hwnd, GA_ROOT) == hwnd
    }
}

fn window_pid(hwnd: HWND) -> Option<u32> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            None
        } else {
            Some(pid)
        }
    }
}

fn process_image_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        if QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_err()
        {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        Some(path)
    }
}

fn exe_basename(path: &str) -> String {
    Path::new(path.trim())
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path.trim())
        .to_ascii_lowercase()
}

fn exe_names_match(expected: &str, image_path: &str) -> bool {
    let actual = Path::new(image_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(image_path)
        .to_ascii_lowercase();
    actual == expected || actual.starts_with(&format!("{expected}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_placement_zone_accepts_known_zones() {
        assert_eq!(validate_placement_zone("left_half").unwrap(), "left_half");
        assert_eq!(validate_placement_zone("right_half").unwrap(), "right_half");
        assert_eq!(validate_placement_zone("maximize").unwrap(), "maximize");
    }

    #[test]
    fn validate_placement_zone_rejects_unknown() {
        assert!(validate_placement_zone("top_left").is_err());
    }

    #[test]
    fn exe_basename_extracts_file_name() {
        assert_eq!(
            exe_basename(r"C:\Program Files\Brave\brave.exe"),
            "brave.exe"
        );
    }

    #[test]
    fn exe_names_match_is_case_insensitive() {
        assert!(exe_names_match(
            "notepad.exe",
            r"C:\Windows\System32\NOTEPAD.EXE"
        ));
    }

    #[test]
    fn placement_status_label_maps_zones() {
        assert_eq!(placement_status_label("left_half"), "left");
        assert_eq!(placement_status_label("right_half"), "right");
    }

    #[test]
    fn window_title_score_fuzzy_matches_brave() {
        let score = title_matches_display("brave", "brave - github");
        assert!(score >= TITLE_MATCH_MIN_RATIO);
    }
}

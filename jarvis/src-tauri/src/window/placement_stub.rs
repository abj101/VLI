//! Non-Windows stubs for window placement APIs.

pub const DEFAULT_MONITOR: &str = "monitor_primary";

pub type WindowSnapshot = ();

pub fn validate_placement_zone(zone: &str) -> Result<&'static str, String> {
    match zone.trim() {
        "left_half" => Ok("left_half"),
        "right_half" => Ok("right_half"),
        "maximize" => Ok("maximize"),
        other => Err(format!("unsupported placement zone `{other}`")),
    }
}

pub fn snapshot_top_level_windows() -> WindowSnapshot {}

pub fn place_window_after_app_launch(
    _exe_path: &str,
    zone: &str,
    _monitor: &str,
    _before: &WindowSnapshot,
) -> Result<(), String> {
    validate_placement_zone(zone)?;
    Err("window placement is only supported on Windows".to_string())
}

pub fn focus_existing_app_window(
    _exe_path: &str,
    _display_name: &str,
    _zone: Option<&str>,
    _monitor: &str,
) -> Result<bool, String> {
    Ok(false)
}

pub fn snap_foreground_window(zone: &str, _monitor: &str) -> Result<(), String> {
    validate_placement_zone(zone)?;
    Err("window placement is only supported on Windows".to_string())
}

pub fn placement_status_label(zone: &str) -> String {
    match zone {
        "left_half" => "left".into(),
        "right_half" => "right".into(),
        "maximize" => "maximized".into(),
        other => other.to_string(),
    }
}

//! Win32 / macOS window placement (zones, snap).

/// App that had focus when listening began. Only the exe: window titles are private
/// and the router only needs the app type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusedWindow {
    pub exe_path: String,
}

#[cfg(windows)]
mod placement_windows;

#[cfg(windows)]
pub use placement_windows::{
    close_app_window, focus_existing_app_window, focus_window_on_side, focused_window,
    has_open_app_window, place_window_after_app_launch, placement_status_label,
    snap_foreground_window, snapshot_top_level_windows, WindowSnapshot, DEFAULT_MONITOR,
};

#[cfg(windows)]
mod text_anchor_windows;
#[cfg(windows)]
pub use text_anchor_windows::{focused_is_address_bar, focused_line_text, focused_text_rect};
#[cfg(not(windows))]
pub fn focused_is_address_bar() -> bool {
    false
}
#[cfg(not(windows))]
pub fn focused_line_text() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
mod placement_macos;

#[cfg(target_os = "macos")]
pub use placement_macos::{
    close_app_window, focus_existing_app_window, focus_window_on_side, focused_window,
    has_open_app_window, place_window_after_app_launch, placement_status_label,
    snap_foreground_window, snapshot_top_level_windows, WindowSnapshot, DEFAULT_MONITOR,
};

#[cfg(not(any(windows, target_os = "macos")))]
mod placement_stub;

#[cfg(not(any(windows, target_os = "macos")))]
pub use placement_stub::{
    close_app_window, focus_existing_app_window, focus_window_on_side, focused_window,
    has_open_app_window, place_window_after_app_launch, placement_status_label,
    snap_foreground_window, snapshot_top_level_windows, WindowSnapshot, DEFAULT_MONITOR,
};

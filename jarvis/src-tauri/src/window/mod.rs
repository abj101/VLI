//! Win32 / macOS window placement (zones, snap).

#[cfg(windows)]
mod placement_windows;

#[cfg(windows)]
pub use placement_windows::{
    focus_existing_app_window, place_window_after_app_launch, placement_status_label,
    snap_foreground_window, snapshot_top_level_windows, WindowSnapshot, DEFAULT_MONITOR,
};

#[cfg(target_os = "macos")]
mod placement_macos;

#[cfg(target_os = "macos")]
pub use placement_macos::{
    focus_existing_app_window, place_window_after_app_launch, placement_status_label,
    snap_foreground_window, snapshot_top_level_windows, WindowSnapshot, DEFAULT_MONITOR,
};

#[cfg(not(any(windows, target_os = "macos")))]
mod placement_stub;

#[cfg(not(any(windows, target_os = "macos")))]
pub use placement_stub::{
    focus_existing_app_window, place_window_after_app_launch, placement_status_label,
    snap_foreground_window, snapshot_top_level_windows, WindowSnapshot, DEFAULT_MONITOR,
};

//! Win32 window placement (zones, snap).

#[cfg(windows)]
mod placement_windows;

#[cfg(windows)]
pub use placement_windows::{
    place_window_after_app_launch, placement_status_label, snap_foreground_window,
    snapshot_top_level_windows, WindowSnapshot, DEFAULT_MONITOR,
};

#[cfg(not(windows))]
mod placement_stub;

#[cfg(not(windows))]
pub use placement_stub::{
    place_window_after_app_launch, placement_status_label, snap_foreground_window,
    snapshot_top_level_windows, WindowSnapshot, DEFAULT_MONITOR,
};

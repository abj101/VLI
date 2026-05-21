//! Windows 11+: disable DWM corner rounding on frameless transparent hosts.
//!
//! DWM `ROUND` uses a different curve/radius than CSS → visible double outline. CSS on
//! `.hud-root` / `.editor-app-shell` is the single source of truth for the visible shape.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::AppHandle;
use tauri::Manager;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};

pub(crate) fn configure_rounded_frame(app: &AppHandle, label: &str) {
    let Some(ww) = app.get_webview_window(label) else {
        return;
    };
    let Ok(handle) = ww.window_handle() else {
        return;
    };
    let hwnd_isize = match handle.as_raw() {
        RawWindowHandle::Win32(h) => h.hwnd.get(),
        _ => return,
    };
    let hwnd = HWND(hwnd_isize as *mut core::ffi::c_void);

    let pref = DWMWCP_DONOTROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::from_ref(&pref).cast(),
            std::mem::size_of_val(&pref) as u32,
        );
    }
}

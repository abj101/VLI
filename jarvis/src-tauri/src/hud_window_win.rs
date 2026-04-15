//! Windows: native rounded corners for the frameless HUD host.

use crate::hud::HUD_WINDOW_LABEL;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::AppHandle;
use tauri::Manager;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};

pub(crate) fn configure_hud_frame(app: &AppHandle) {
    let Some(ww) = app.get_webview_window(HUD_WINDOW_LABEL) else {
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

    // HUD client uses CSS clipping; DWM supplies native Win11 rounded corners.
    let pref = DWMWCP_ROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::from_ref(&pref).cast(),
            std::mem::size_of_val(&pref) as u32,
        );
    }
}

//! Windows: native rounded corners for the frameless editor host so DWM does not paint a square halo.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::AppHandle;
use tauri::Manager;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL,
};

const EDITOR_LABEL: &str = "editor";

pub(crate) fn configure_editor_frame(app: &AppHandle) {
    let Some(ww) = app.get_webview_window(EDITOR_LABEL) else {
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

    // Pairs with `--editor-shell-radius` in EditorRoot.css: `ROUND` is a larger system arc than our clip.
    let pref = DWMWCP_ROUNDSMALL;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::from_ref(&pref).cast(),
            std::mem::size_of_val(&pref) as u32,
        );
    }
}

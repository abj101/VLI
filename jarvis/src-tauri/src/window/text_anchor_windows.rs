//! Screen rect of the focused text box, so the dictation HUD can sit beside it.

use super::placement_windows::{is_own_pid, window_pid};
use crate::hud::PxRect;
use std::cell::OnceCell;
use windows::core::Interface;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetUBound, SafeArrayUnaccessData,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    AccessibleObjectFromWindow, CUIAutomation, IAccessible, IUIAutomation, IUIAutomation2,
    IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationTextPattern2, IUIAutomationValuePattern,
    IUIAutomationTextRange, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start,
    TextUnit_Character, TextUnit_Line, UIA_DocumentControlTypeId, UIA_EditControlTypeId,
    UIA_TextPattern2Id, UIA_TextPatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId,
    GA_ROOT, GUITHREADINFO, OBJID_CARET,
};

thread_local! {
    // One automation client per polling thread (COM apartment is per-thread).
    static UIA: OnceCell<Option<IUIAutomation>> = const { OnceCell::new() };
}

fn uia() -> Option<IUIAutomation> {
    UIA.with(|cell| {
        cell.get_or_init(|| unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let uia: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            if let Ok(u2) = uia.cast::<IUIAutomation2>() {
                let _ = u2.SetConnectionTimeout(500); // hung apps must not stall the follower
            }
            Some(uia)
        })
        .clone()
    })
}

/// Whether the focused UIA edit control is a browser address/search bar.
// ponytail: Browser UIA identifiers are unverified; keep this detection narrow.
pub fn focused_is_address_bar() -> bool {
    if focus_is_own() {
        return false;
    }
    let Some(automation) = uia() else {
        return false;
    };
    let Ok(element) = (unsafe { automation.GetFocusedElement() }) else {
        return false;
    };
    unsafe {
        if element.CurrentControlType().ok() != Some(UIA_EditControlTypeId) {
            return false;
        }

        let is_chromium = element
            .CurrentClassName()
            .ok()
            .is_some_and(|class_name| class_name == "OmniboxViewViews");
        let is_firefox = element
            .CurrentAutomationId()
            .ok()
            .is_some_and(|automation_id| {
                automation_id
                    .to_string()
                    .eq_ignore_ascii_case("urlbar-input")
            });
        let is_named_address_bar = element.CurrentName().ok().is_some_and(|name| {
            let name = name.to_string().to_lowercase();
            name.contains("address and search bar") || name.contains("search or enter")
        });

        is_chromium || is_firefox || is_named_address_bar
    }
}

/// Same guard as `focused_text_rect`: a UIA query into our own UI thread can block on it.
fn focus_is_own() -> bool {
    gui_thread_info()
        .map(|info| unsafe { GetAncestor(info.hwndActive, GA_ROOT) })
        .and_then(window_pid)
        .is_none_or(|pid| is_own_pid(pid, std::process::id()))
}

/// Text of the line the caret is on in the focused text field. `None` when the app exposes no
/// UIA text (then dictation can't see list markers and leaves numbering to the app).
pub fn focused_line_text() -> Option<String> {
    if focus_is_own() {
        return None;
    }
    unsafe {
        let el = uia()?.GetFocusedElement().ok()?;
        let range = el
            .GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
            .ok()
            .and_then(|p| p.GetCaretRange(&mut BOOL::default()).ok())
            .or_else(|| {
                let p: IUIAutomationTextPattern = el.GetCurrentPatternAs(UIA_TextPatternId).ok()?;
                p.GetSelection().ok()?.GetElement(0).ok()
            })?;
        range.ExpandToEnclosingUnit(TextUnit_Line).ok()?;
        Some(range.GetText(500).ok()?.to_string())
    }
}

fn gui_thread_info() -> Option<GUITHREADINFO> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(GetWindowThreadProcessId(fg, None), &mut info).ok()?;
        Some(info)
    }
}

fn tuple(r: RECT) -> PxRect {
    (r.left, r.top, r.right, r.bottom)
}

/// Where the dictation HUD should anchor (physical px): the text caret, else the focused text
/// box, plus the work area of its monitor. `None` for Jarvis itself, non-text focus,
/// elevated/unreadable windows, or a page-sized box with no readable caret.
pub fn focused_text_rect() -> Option<(PxRect, PxRect)> {
    let info = gui_thread_info()?;
    let root = unsafe { GetAncestor(info.hwndActive, GA_ROOT) };
    if is_own_pid(window_pid(root)?, std::process::id()) {
        return None;
    }
    // Only text fields count: a focused button, tab or list must not pull the HUD over.
    let field = uia().and_then(|u| unsafe {
        let el = u.GetFocusedElement().ok()?;
        let kind = el.CurrentControlType().ok()?;
        (kind == UIA_EditControlTypeId || kind == UIA_DocumentControlTypeId).then_some(el)
    });
    if field.as_ref().is_some_and(|el| unsafe {
        el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .and_then(|value| value.CurrentIsReadOnly())
            .is_ok_and(|read_only| read_only.as_bool())
    }) {
        return None;
    }
    let edit_box = field.as_ref().is_some_and(|el| unsafe {
        el.CurrentControlType().ok() == Some(UIA_EditControlTypeId)
    });
    // Selected text, then TextPattern2 (WinUI, Word), then the MSAA caret Chromium/Electron keep for IMEs, then Win32.
    let caret = field.as_ref().and_then(uia_caret_rect)
        .or_else(|| msaa_caret(&info))
        .or_else(|| caret_rect(&info));
    let selection = field.as_ref().filter(|_| edit_box || caret.is_some()).and_then(selection_rect);
    let rect = match selection.or(caret) {
        Some(caret) => caret,
        None => {
            let boxed = field.filter(|_| edit_box)
                .and_then(|el| unsafe { el.CurrentBoundingRectangle().ok() })?;
            let work = work_area(&boxed)?;
            // Beside a page-sized box is off-screen anyway.
            let big = |a: i32, b: i32| a * 10 > b * 6;
            if big(boxed.right - boxed.left, work.right - work.left)
                || big(boxed.bottom - boxed.top, work.bottom - work.top)
            {
                return None;
            }
            boxed
        }
    };
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return None;
    }
    Some((tuple(rect), tuple(work_area(&rect)?)))
}

/// Caret from MSAA `OBJID_CARET` on the focused window.
fn msaa_caret(info: &GUITHREADINFO) -> Option<RECT> {
    let hwnd = if info.hwndFocus.0.is_null() {
        info.hwndActive
    } else {
        info.hwndFocus
    };
    unsafe {
        let mut acc: Option<IAccessible> = None;
        AccessibleObjectFromWindow(
            hwnd,
            OBJID_CARET.0 as u32,
            &IAccessible::IID,
            &mut acc as *mut _ as *mut _,
        )
        .ok()?;
        let (mut l, mut t, mut w, mut h) = (0, 0, 0, 0);
        acc?.accLocation(&mut l, &mut t, &mut w, &mut h, &VARIANT::from(0i32))
            .ok()?;
        (h > 0 && (l, t) != (0, 0)).then_some(RECT {
            left: l,
            top: t,
            right: l + w.max(1),
            bottom: t + h,
        })
    }
}

/// Caret from UIA TextPattern2. A collapsed range often has no rectangles, so measure the
/// next character, or the previous one's right edge at the end of the text.
fn uia_caret_rect(el: &IUIAutomationElement) -> Option<RECT> {
    unsafe {
        let pattern: IUIAutomationTextPattern2 = el.GetCurrentPatternAs(UIA_TextPattern2Id).ok()?;
        let mut active = BOOL::default();
        let range = pattern.GetCaretRange(&mut active).ok()?;
        if !active.as_bool() { return None; }
        if let Some(r) = range_rect(&range) {
            return Some(r);
        }
        let next = range.Clone().ok()?;
        next.ExpandToEnclosingUnit(TextUnit_Character).ok()?;
        if let Some(r) = range_rect(&next) {
            return Some(RECT {
                right: r.left + 1,
                ..r
            });
        }
        range
            .MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1)
            .ok()?;
        range_rect(&range).map(|r| RECT {
            left: r.right - 1,
            ..r
        })
    }
}

/// Bounds of a text range: the union of its per-line rectangles
/// (UIA returns `[left, top, width, height, ...]`).
unsafe fn range_rect(range: &IUIAutomationTextRange) -> Option<RECT> {
    let sa = range.GetBoundingRectangles().ok()?;
    if sa.is_null() {
        return None;
    }
    let mut data = std::ptr::null_mut();
    let n = SafeArrayGetUBound(sa, 1).map_or(0, |ub| (ub + 1) as usize / 4 * 4);
    let rect = (n >= 4 && SafeArrayAccessData(sa, &mut data).is_ok()).then(|| {
        let v = std::slice::from_raw_parts(data as *const f64, n);
        let r = v.chunks(4).fold(None::<RECT>, |acc, q| {
            let q = RECT {
                left: q[0] as i32,
                top: q[1] as i32,
                right: (q[0] + q[2].max(1.0)) as i32,
                bottom: (q[1] + q[3]) as i32,
            };
            Some(acc.map_or(q, |a| RECT {
                left: a.left.min(q.left),
                top: a.top.min(q.top),
                right: a.right.max(q.right),
                bottom: a.bottom.max(q.bottom),
            }))
        });
        let _ = SafeArrayUnaccessData(sa);
        r
    });
    let _ = SafeArrayDestroy(sa);
    rect.flatten().filter(|r| r.bottom > r.top)
}

/// Bounds of a non-empty selection (Ctrl+A etc.), so the HUD sits beside the selected text.
fn selection_rect(el: &IUIAutomationElement) -> Option<RECT> {
    unsafe {
        let pattern: IUIAutomationTextPattern = el.GetCurrentPatternAs(UIA_TextPatternId).ok()?;
        let range = pattern.GetSelection().ok()?.GetElement(0).ok()?;
        let empty = range
            .CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                &range,
                TextPatternRangeEndpoint_End,
            )
            .ok()?
            == 0;
        if empty {
            return None;
        }
        range_rect(&range)
    }
}


fn caret_rect(info: &GUITHREADINFO) -> Option<RECT> {
    if info.hwndCaret.0.is_null() {
        return None;
    }
    let (mut tl, mut br) = (
        POINT {
            x: info.rcCaret.left,
            y: info.rcCaret.top,
        },
        POINT {
            x: info.rcCaret.right,
            y: info.rcCaret.bottom,
        },
    );
    unsafe {
        if !ClientToScreen(info.hwndCaret, &mut tl).as_bool()
            || !ClientToScreen(info.hwndCaret, &mut br).as_bool()
        {
            return None;
        }
    }
    Some(RECT {
        left: tl.x,
        top: tl.y,
        right: br.x.max(tl.x + 1),
        bottom: br.y,
    })
}

fn work_area(rect: &RECT) -> Option<RECT> {
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetMonitorInfoW(MonitorFromRect(rect, MONITOR_DEFAULTTONEAREST), &mut mi)
            .ok()
            .ok()?
    };
    Some(mi.rcWork)
}

#[cfg(test)]
mod probe {
    use super::*;

    /// Manual diagnostic for a new app: focus a text field, then run
    /// `cargo test --lib text_anchor_probe -- --ignored --nocapture` and switch to that app within 1 s.
    #[test]
    #[ignore]
    fn text_anchor_probe() {
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let info = gui_thread_info();
            let el = uia().and_then(|u| unsafe { u.GetFocusedElement().ok() });
            println!(
                "class={:?} line={:?} selection={:?} tp2={:?} msaa={:?} classic={:?} final={:?}",
                el.as_ref()
                    .and_then(|e| unsafe { e.CurrentClassName().ok() }),
                focused_line_text(),
                el.as_ref().and_then(selection_rect),
                el.as_ref().and_then(uia_caret_rect),
                info.as_ref().and_then(msaa_caret),
                info.as_ref().and_then(caret_rect),
                focused_text_rect(),
            );
        }
    }
}

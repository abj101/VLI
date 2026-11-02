//! Unicode keyboard injection for dictation (Windows SendInput).

/// Max `INPUT` records per `SendInput` call (Windows cap).
#[cfg(windows)]
const SEND_INPUT_BATCH_EVENTS: usize = 64;

#[cfg(windows)]
fn unicode_inputs(text: &str) -> Vec<windows::Win32::UI::Input::KeyboardAndMouse::INPUT> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        VIRTUAL_KEY, VK_RETURN,
    };
    let mut inputs = Vec::new();
    for unit in text.encode_utf16() {
        let newline = unit == b'\n' as u16 || unit == b'\r' as u16;
        for up in [false, true] {
            let mut flags = if newline {
                Default::default()
            } else {
                KEYEVENTF_UNICODE
            };
            if up {
                flags |= KEYEVENTF_KEYUP;
            }
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: if newline { VK_RETURN } else { VIRTUAL_KEY(0) },
                        wScan: if newline { 0 } else { unit },
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
    }
    inputs
}

#[cfg(windows)]
pub fn type_unicode(text: &str) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};
    for batch in unicode_inputs(text).chunks(SEND_INPUT_BATCH_EVENTS) {
        let sent = unsafe { SendInput(batch, std::mem::size_of::<INPUT>() as i32) };
        if sent != batch.len() as u32 {
            return Err(format!("SendInput accepted {sent}/{} events", batch.len()));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn send_virtual_key_burst(
    vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY,
    count: usize,
) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    };

    let mut inputs = Vec::with_capacity(count * 2);
    for _ in 0..count {
        for key_up in [false, true] {
            let flags = if key_up {
                KEYEVENTF_KEYUP
            } else {
                Default::default()
            };
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: 0,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        return Err(format!(
            "SendInput accepted {sent}/{} backspace events",
            inputs.len()
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn send_virtual_key(
    vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY,
) -> Result<(), String> {
    send_virtual_key_burst(vk, 1)
}

#[cfg(windows)]
pub(crate) fn send_virtual_key_chord(keys: &[u16]) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };

    let key_input = |key: u16, up: bool| {
        let mut flags = if up {
            KEYEVENTF_KEYUP
        } else {
            KEYBD_EVENT_FLAGS::default()
        };
        // Nav cluster / Win keys: without this, Shift+arrows arrive as numpad keys.
        if matches!(key, 0x21..=0x28 | 0x2D | 0x2E | 0x5B | 0x5C | 0x6F | 0x90) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(key),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    };

    let inputs: Vec<_> = keys
        .iter()
        .map(|&k| key_input(k, false))
        .chain(keys.iter().rev().map(|&k| key_input(k, true)))
        .collect();
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        let releases: Vec<_> = keys.iter().rev().map(|&k| key_input(k, true)).collect();
        unsafe { SendInput(&releases, std::mem::size_of::<INPUT>() as i32) };
        return Err(format!(
            "SendInput accepted {sent}/{} key events",
            inputs.len()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn type_unicode(_text: &str) -> Result<(), String> {
    Err("dictation typing is Windows-only for now".into())
}

#[cfg(not(windows))]
pub(crate) fn send_virtual_key(_vk: u16) -> Result<(), String> {
    Err("not supported on this platform".into())
}

#[cfg(not(windows))]
pub(crate) fn send_virtual_key_chord(_keys: &[u16]) -> Result<(), String> {
    Err("not supported on this platform".into())
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn unicode_input_includes_surrogate_pair() {
        assert_eq!(super::unicode_inputs("a\u{1f600}").len(), 6);
    }
}

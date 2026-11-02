//! Unicode keyboard injection for dictation (Windows SendInput).

/// Max `INPUT` records per `SendInput` call (Windows cap).
#[cfg(windows)]
const SEND_INPUT_BATCH_EVENTS: usize = 64;

#[cfg(any(windows, test))]
const TEXT_INPUT_MODIFIERS: [i32; 5] = [0x10, 0x11, 0x12, 0x5B, 0x5C];

#[cfg(any(windows, test))]
fn wait_for_modifier_release_with(mut key_state: impl FnMut(i32) -> i16, mut pause: impl FnMut()) {
    // Only the high bit means held. The low bit records an earlier press.
    while TEXT_INPUT_MODIFIERS.iter().any(|&key| key_state(key) < 0) {
        pause();
    }
}

/// Dictation must not become Ctrl/Alt/Win shortcuts when its hotkey interrupts typing.
#[cfg(windows)]
pub(crate) fn wait_for_modifiers_released() {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    // Poll physical state, not the shortcut Released callback: the callback can be
    // synchronously flushing the final utterance while the user still holds Alt.
    wait_for_modifier_release_with(
        |key| unsafe { GetAsyncKeyState(key) },
        || std::thread::sleep(std::time::Duration::from_millis(5)),
    );
}

#[cfg(not(windows))]
pub(crate) fn wait_for_modifiers_released() {}

/// A consumed Alt hotkey must not leave a bare Alt-up that activates the app's menu.
#[cfg(windows)]
pub(crate) fn mask_alt_menu_activation() -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY;
    // VK_E8 is unassigned; unlike Escape or Ctrl it has no editing action of its own.
    send_virtual_key(VIRTUAL_KEY(0xE8))
}

#[cfg(windows)]
fn unicode_inputs(text: &str) -> Vec<windows::Win32::UI::Input::KeyboardAndMouse::INPUT> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        VIRTUAL_KEY, VK_RETURN, VK_SHIFT,
    };

    let mut inputs = Vec::new();
    let mut skip_lf = false;
    for unit in text.encode_utf16() {
        if skip_lf && unit == b'\n' as u16 {
            skip_lf = false;
            continue;
        }
        skip_lf = unit == b'\r' as u16;

        let newline = unit == b'\n' as u16 || unit == b'\r' as u16;
        if newline {
            for (vk, key_up) in [
                (VK_SHIFT, false),
                (VK_RETURN, false),
                (VK_RETURN, true),
                (VK_SHIFT, true),
            ] {
                inputs.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: vk,
                            wScan: 0,
                            dwFlags: if key_up {
                                KEYEVENTF_KEYUP
                            } else {
                                Default::default()
                            },
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                });
            }
            continue;
        }

        for key_up in [false, true] {
            let mut flags = KEYEVENTF_UNICODE;
            if key_up {
                flags |= KEYEVENTF_KEYUP;
            }
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: unit,
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
    fn newline_is_shift_enter_and_crlf_is_one_newline() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_RETURN, VK_SHIFT};
        let inputs = super::unicode_inputs("\r\n\n");
        assert_eq!(inputs.len(), 8);
        for (input, (key, up)) in inputs.iter().zip(
            [
                (VK_SHIFT, false),
                (VK_RETURN, false),
                (VK_RETURN, true),
                (VK_SHIFT, true),
            ]
            .into_iter()
            .cycle(),
        ) {
            let key_input = unsafe { input.Anonymous.ki };
            assert_eq!(key_input.wVk, key);
            assert_eq!(key_input.dwFlags.contains(KEYEVENTF_KEYUP), up);
        }
    }
    #[test]
    fn unicode_input_includes_surrogate_pair() {
        assert_eq!(super::unicode_inputs("a\u{1f600}").len(), 6);
    }
}

#[cfg(test)]
mod modifier_tests {
    use std::cell::Cell;

    #[test]
    fn final_text_waits_until_every_shortcut_modifier_is_released() {
        for held_key in super::TEXT_INPUT_MODIFIERS {
            let pauses = Cell::new(0);
            super::wait_for_modifier_release_with(
                |key| {
                    if key == held_key && pauses.get() < 3 {
                        i16::MIN
                    } else {
                        0
                    }
                },
                || pauses.set(pauses.get() + 1),
            );
            assert_eq!(pauses.get(), 3, "modifier {held_key:#x}");
        }
    }

    #[test]
    fn a_previous_press_does_not_block_the_next_dictation_session() {
        super::wait_for_modifier_release_with(|_| 1, || panic!("released keys must not block"));
    }

    #[test]
    fn unmodified_text_is_not_delayed() {
        super::wait_for_modifier_release_with(|_| 0, || panic!("no modifier is held"));
    }
}

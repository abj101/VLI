//! Unicode keyboard injection for dictation (Windows SendInput).

/// Max `INPUT` records per `SendInput` call (Windows cap).
#[cfg(windows)]
const SEND_INPUT_BATCH_EVENTS: usize = 64;

/// Backspaces per batch (`down` + `up` = 2 events each).
#[cfg(windows)]
const BACKSPACES_PER_BATCH: usize = SEND_INPUT_BATCH_EVENTS / 2;

#[cfg(windows)]
pub fn type_unicode(text: &str) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;

    if text.is_empty() {
        return Ok(());
    }

    for ch in text.chars() {
        if ch == '\n' || ch == '\r' {
            send_virtual_key(VK_RETURN)?;
            continue;
        }
        send_unicode_char(ch)?;
    }
    Ok(())
}

#[cfg(windows)]
pub fn send_backspaces(count: usize) -> Result<(), String> {
    use std::thread;
    use std::time::Duration;
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_BACK;

    if count == 0 {
        return Ok(());
    }

    let mut remaining = count;
    while remaining > 0 {
        let batch = remaining.min(BACKSPACES_PER_BATCH);
        send_virtual_key_burst(VK_BACK, batch)?;
        remaining -= batch;
        if remaining > 0 {
            // Let the focused control drain the queue before the next burst.
            thread::sleep(Duration::from_millis(1));
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
fn send_unicode_char(ch: char) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        VIRTUAL_KEY,
    };

    let code = ch as u16;
    for key_up in [false, true] {
        let flags = if key_up {
            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
        } else {
            KEYEVENTF_UNICODE
        };
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: code,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        if sent == 0 {
            return Err("SendInput failed".into());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn send_virtual_key(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> Result<(), String> {
    send_virtual_key_burst(vk, 1)
}

#[cfg(not(windows))]
pub fn type_unicode(_text: &str) -> Result<(), String> {
    Err("dictation typing is Windows-only for now".into())
}

#[cfg(not(windows))]
pub fn send_backspaces(_count: usize) -> Result<(), String> {
    Err("dictation typing is Windows-only for now".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backspace_batch_size_respects_sendinput_cap() {
        #[cfg(windows)]
        assert_eq!(BACKSPACES_PER_BATCH, 32);
    }
}

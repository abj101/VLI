//! Cross-platform keyboard injection for automation `send_keys` actions.

use crate::process::hidden_command;

/// One step of a parsed SendKeys string: a VK chord (modifiers first) or plain text.
#[cfg(windows)]
#[derive(Debug, PartialEq, Eq)]
enum WinKeyOp {
    Chord(Vec<u16>),
    Text(String),
}

#[cfg(windows)]
const VK_SHIFT: u16 = 0x10;
#[cfg(windows)]
const VK_CONTROL: u16 = 0x11;
#[cfg(windows)]
const VK_MENU: u16 = 0x12;

/// Parse .NET SendKeys syntax into SendInput ops. `None` = needs the PowerShell
/// fallback (`(...)` groups, `{KEY n}` repeats, modified punctuation, unknown names).
#[cfg(windows)]
fn parse_windows_send_keys(keys: &str) -> Option<Vec<WinKeyOp>> {
    let mut ops = Vec::new();
    let mut mods: Vec<u16> = Vec::new();
    let mut chars = keys.chars();
    while let Some(ch) = chars.next() {
        let key = match ch {
            '^' | '+' | '%' => {
                let vk = match ch {
                    '^' => VK_CONTROL,
                    '+' => VK_SHIFT,
                    _ => VK_MENU,
                };
                if !mods.contains(&vk) {
                    mods.push(vk);
                }
                continue;
            }
            '(' | ')' => return None,
            '~' => KeyTok::Vk(0x0D),
            '{' => {
                // `{}}` and `{{}` are the escaped braces.
                let rest = chars.as_str();
                let first = rest.chars().next()?.len_utf8();
                let end = rest[first..].find('}')? + first;
                let token = &rest[..end];
                chars = rest[end + 1..].chars();
                let mut it = token.chars();
                match (it.next(), it.next()) {
                    (Some(c), None) => KeyTok::Char(c),
                    _ => KeyTok::Vk(windows_vk_for_name(token)?),
                }
            }
            c => KeyTok::Char(c),
        };
        match key {
            KeyTok::Char(c) if mods.is_empty() => match ops.last_mut() {
                Some(WinKeyOp::Text(s)) => s.push(c),
                _ => ops.push(WinKeyOp::Text(c.to_string())),
            },
            KeyTok::Char(c) => {
                let vk = match c {
                    'a'..='z' | 'A'..='Z' | '0'..='9' => c.to_ascii_uppercase() as u16,
                    ' ' => 0x20,
                    _ => return None,
                };
                if c.is_ascii_uppercase() && !mods.contains(&VK_SHIFT) {
                    mods.push(VK_SHIFT);
                }
                mods.push(vk);
                ops.push(WinKeyOp::Chord(std::mem::take(&mut mods)));
            }
            KeyTok::Vk(vk) => {
                mods.push(vk);
                ops.push(WinKeyOp::Chord(std::mem::take(&mut mods)));
            }
        }
    }
    // Trailing modifiers with nothing to apply to: let .NET decide.
    if !mods.is_empty() {
        return None;
    }
    Some(ops)
}

#[cfg(windows)]
enum KeyTok {
    Char(char),
    Vk(u16),
}

#[cfg(windows)]
fn windows_vk_for_name(name: &str) -> Option<u16> {
    let upper = name.to_ascii_uppercase();
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        return (1..=24).contains(&n).then_some(0x70 + n - 1);
    }
    Some(match upper.as_str() {
        "ENTER" => 0x0D,
        "TAB" => 0x09,
        "ESC" | "ESCAPE" => 0x1B,
        "BACKSPACE" | "BS" | "BKSP" => 0x08,
        "DELETE" | "DEL" => 0x2E,
        "INSERT" | "INS" => 0x2D,
        "HOME" => 0x24,
        "END" => 0x23,
        "PGUP" => 0x21,
        "PGDN" => 0x22,
        "LEFT" => 0x25,
        "UP" => 0x26,
        "RIGHT" => 0x27,
        "DOWN" => 0x28,
        "CAPSLOCK" => 0x14,
        "NUMLOCK" => 0x90,
        "SCROLLLOCK" => 0x91,
        "PRTSC" => 0x2C,
        "ADD" => 0x6B,
        "SUBTRACT" => 0x6D,
        "MULTIPLY" => 0x6A,
        "DIVIDE" => 0x6F,
        _ => return None,
    })
}

#[cfg(windows)]
pub fn send_keys(keys: &str) -> Result<(), String> {
    let Some(ops) = parse_windows_send_keys(keys) else {
        return send_keys_powershell(keys);
    };
    for op in ops {
        match op {
            WinKeyOp::Chord(vks) => crate::input::type_text::send_virtual_key_chord(&vks),
            WinKeyOp::Text(text) => crate::input::type_text::type_unicode(&text),
        }
        .map_err(|e| format!("failed to send keys `{keys}`: {e}"))?;
    }
    Ok(())
}

#[cfg(windows)]
fn send_keys_powershell(keys: &str) -> Result<(), String> {
    let escaped = keys.replace('\'', "''");
    let command = format!(
        "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{escaped}')"
    );
    let status = hidden_command("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(command)
        .status()
        .map_err(|e| format!("failed to send keys `{keys}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "failed to send keys `{keys}`: powershell exited with {status}"
        ))
    }
}

#[cfg(target_os = "macos")]
pub fn send_keys(keys: &str) -> Result<(), String> {
    let script = build_macos_send_keys_script(keys)?;
    let status = hidden_command("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .map_err(|e| format!("failed to send keys `{keys}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "failed to send keys `{keys}`: osascript exited with {status} (grant Accessibility permission in System Settings)"
        ))
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn send_keys(keys: &str) -> Result<(), String> {
    let _ = keys;
    Err("send_keys is not supported on this platform".to_string())
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MacModifiers {
    command: bool,
    shift: bool,
    option: bool,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum MacKeyOp {
    Keystroke { text: String, modifiers: MacModifiers },
    KeyCode { code: u16, modifiers: MacModifiers },
}

#[cfg(target_os = "macos")]
fn build_macos_send_keys_script(keys: &str) -> Result<String, String> {
    let ops = parse_send_keys_ops(keys)?;
    if ops.is_empty() {
        return Err("SendKeys keys cannot be empty".to_string());
    }
    let mut lines = vec!["tell application \"System Events\"".to_string()];
    for op in ops {
        lines.push(format!("  {}", op_to_applescript(&op)));
    }
    lines.push("end tell".to_string());
    Ok(lines.join("\n"))
}

#[cfg(target_os = "macos")]
fn op_to_applescript(op: &MacKeyOp) -> String {
    match op {
        MacKeyOp::Keystroke { text, modifiers } => {
            let mods = modifiers_applescript(modifiers);
            if mods.is_empty() {
                format!("keystroke {}", applescript_string(text))
            } else {
                format!(
                    "keystroke {} using {}",
                    applescript_string(text),
                    mods
                )
            }
        }
        MacKeyOp::KeyCode { code, modifiers } => {
            let mods = modifiers_applescript(modifiers);
            if mods.is_empty() {
                format!("key code {code}")
            } else {
                format!("key code {code} using {}", mods)
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn modifiers_applescript(modifiers: &MacModifiers) -> String {
    let mut parts = Vec::new();
    if modifiers.command {
        parts.push("command down");
    }
    if modifiers.shift {
        parts.push("shift down");
    }
    if modifiers.option {
        parts.push("option down");
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", parts.join(", "))
    }
}

#[cfg(target_os = "macos")]
fn applescript_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(target_os = "macos")]
fn parse_send_keys_ops(keys: &str) -> Result<Vec<MacKeyOp>, String> {
    let mut ops = Vec::new();
    let mut modifiers = MacModifiers::default();
    let mut literal = String::new();
    let chars: Vec<char> = keys.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '^' => {
                flush_literal(&mut literal, &mut ops, &mut modifiers)?;
                modifiers.command = true;
            }
            '+' => {
                flush_literal(&mut literal, &mut ops, &mut modifiers)?;
                modifiers.shift = true;
            }
            '%' => {
                flush_literal(&mut literal, &mut ops, &mut modifiers)?;
                modifiers.option = true;
            }
            '{' => {
                flush_literal(&mut literal, &mut ops, &mut modifiers)?;
                let end = keys[i + 1..]
                    .find('}')
                    .ok_or_else(|| format!("unclosed SendKeys group in `{keys}`"))?;
                let token = &keys[i + 1..i + 1 + end];
                ops.push(special_key_op(token, &modifiers)?);
                modifiers = MacModifiers::default();
                i += end + 1;
            }
            _ => literal.push(ch),
        }
        i += 1;
    }
    flush_literal(&mut literal, &mut ops, &mut modifiers)?;
    Ok(ops)
}

#[cfg(target_os = "macos")]
fn flush_literal(
    literal: &mut String,
    ops: &mut Vec<MacKeyOp>,
    modifiers: &mut MacModifiers,
) -> Result<(), String> {
    if literal.is_empty() {
        return Ok(());
    }
    let text = std::mem::take(literal);
    if text.len() == 1 {
        ops.push(MacKeyOp::Keystroke {
            text,
            modifiers: std::mem::take(modifiers),
        });
    } else {
        for ch in text.chars() {
            ops.push(MacKeyOp::Keystroke {
                text: ch.to_string(),
                modifiers: modifiers.clone(),
            });
        }
        *modifiers = MacModifiers::default();
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn special_key_op(token: &str, modifiers: &MacModifiers) -> Result<MacKeyOp, String> {
    let code = match token.to_ascii_uppercase().as_str() {
        "ENTER" | "RETURN" => 36,
        "TAB" => 48,
        "ESC" | "ESCAPE" => 53,
        "BACKSPACE" | "BS" => 51,
        "DELETE" | "DEL" => 117,
        "HOME" => 115,
        "END" => 119,
        "LEFT" => 123,
        "RIGHT" => 124,
        "UP" => 126,
        "DOWN" => 125,
        other => {
            return Err(format!("unsupported SendKeys token `{{{other}}}` on macOS"));
        }
    };
    Ok(MacKeyOp::KeyCode {
        code,
        modifiers: modifiers.clone(),
    })
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::{parse_windows_send_keys, WinKeyOp::*};

    fn chord(vks: &[u16]) -> super::WinKeyOp {
        Chord(vks.to_vec())
    }

    #[test]
    fn parses_shortcut_tool_keys() {
        let p = |k| parse_windows_send_keys(k).unwrap_or_else(|| panic!("{k}"));
        assert_eq!(p("^c"), vec![chord(&[0x11, b'C' as u16])]);
        assert_eq!(p("^+t"), vec![chord(&[0x11, 0x10, b'T' as u16])]);
        assert_eq!(p("^{TAB}"), vec![chord(&[0x11, 0x09])]);
        assert_eq!(p("^+{TAB}"), vec![chord(&[0x11, 0x10, 0x09])]);
        assert_eq!(p("{F5}"), vec![chord(&[0x74])]);
        assert_eq!(p("%{F4}"), vec![chord(&[0x12, 0x73])]);
        assert_eq!(p("%{LEFT}"), vec![chord(&[0x12, 0x25])]);
        assert_eq!(p("%{right}"), vec![chord(&[0x12, 0x27])]);
        assert_eq!(p("^T"), vec![chord(&[0x11, 0x10, b'T' as u16])]);
    }

    #[test]
    fn literal_text_and_escapes() {
        let p = |k| parse_windows_send_keys(k).unwrap();
        assert_eq!(p("hi there"), vec![Text("hi there".into())]);
        assert_eq!(
            p("^ab~"),
            vec![chord(&[0x11, b'A' as u16]), Text("b".into()), chord(&[0x0D])]
        );
        assert_eq!(p("{+}{{}{}}"), vec![Text("+{}".into())]);
        assert_eq!(p("{é}x"), vec![Text("éx".into())]);
    }

    #[test]
    fn unsupported_grammar_falls_back() {
        for k in ["+(ab)", "{LEFT 3}", "^-", "{NOPE}", "^", "{F5", "{}"] {
            assert_eq!(parse_windows_send_keys(k), None, "{k}");
        }
    }

    /// Injects F15 into the focused window. Run:
    /// `cargo test --lib send_keys_latency -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn send_keys_latency() {
        use std::time::Instant;
        for (label, f) in [
            ("powershell", super::send_keys_powershell as fn(&str) -> Result<(), String>),
            ("sendinput", super::send_keys),
        ] {
            let mut ms = Vec::new();
            for _ in 0..5 {
                let t = Instant::now();
                f("{F15}").unwrap();
                ms.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            println!("{label}: {ms:.2?} ms");
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::{build_macos_send_keys_script, parse_send_keys_ops, MacKeyOp, MacModifiers};

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_cmd_v_keystroke() {
        let ops = parse_send_keys_ops("^v").expect("parse");
        assert_eq!(
            ops,
            vec![MacKeyOp::Keystroke {
                text: "v".into(),
                modifiers: MacModifiers {
                    command: true,
                    ..Default::default()
                }
            }]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_literal_text() {
        let ops = parse_send_keys_ops("hello").expect("parse");
        assert_eq!(ops.len(), 5);
        assert!(ops.iter().all(|op| matches!(op, MacKeyOp::Keystroke { .. })));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn build_script_for_cmd_n() {
        let script = build_macos_send_keys_script("^n").expect("script");
        assert!(script.contains("tell application \"System Events\""));
        assert!(script.contains("keystroke \"n\" using {command down}"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn build_script_for_enter() {
        let script = build_macos_send_keys_script("{ENTER}").expect("script");
        assert!(script.contains("key code 36"));
    }
}

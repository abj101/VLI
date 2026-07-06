//! Cross-platform keyboard injection for automation `send_keys` actions.

use crate::process::hidden_command;

#[cfg(windows)]
pub fn send_keys(keys: &str) -> Result<(), String> {
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

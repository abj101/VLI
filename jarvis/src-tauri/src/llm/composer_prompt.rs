//! Build prompts for the on-device command composer model.

/// Compact one-line-per-action catalog for the composer system prompt.
pub const ACTION_CATALOG: &str = r#"open_target(target, placement?) — prefer for apps/sites; placement: left_half, right_half, maximize
place_window(zone) — snap active window (left_half, right_half, maximize)
send_keys(keys) — type or hotkeys (e.g. ^v paste, ^n new)
set_clipboard(text) — put text on clipboard before paste
speak(text) — text-to-speech
http_get(url) — fetch URL body
get_clipboard({}) — read clipboard text
if_else(condition, then[], else[]) — branch on text
wait(ms) — pause
sub_prompt(prompt) — voice follow-up"#;

#[cfg(target_os = "macos")]
const PLATFORM_RULES: &str = r#"Platform: macOS. Use TextEdit for simple notes (not Notepad). App targets use display names (e.g. TextEdit, Chrome). Shortcut notation: ^ means Command (e.g. ^n new document, ^v paste)."#;

#[cfg(not(target_os = "macos"))]
const PLATFORM_RULES: &str = r#"Platform: Windows. Use Notepad for simple notes. Shortcut notation: ^ means Ctrl (e.g. ^n new document, ^v paste)."#;

const TEMPLATE_RULES: &str = r#"Template vars: {{remainder}} (words after prefix trigger), {{last_result}}, {{step_N}}.
Use match_mode "prefix" when words after the trigger are input; "phrase" for fixed phrases.
Put the exact voice activation phrase in "trigger" — only the words the user speaks to start the command, never the action steps.
Optional "trigger_phrases" holds extra aliases; when omitted, copy "trigger" is enough.
When I say <trigger> then <actions>: <trigger> goes in "trigger" only; <actions> become the action chain. Never duplicate the trigger as an action unless the user asked twice.
"then" and "and then" separate steps in the action list.
Timers → wait(ms); e.g. "30 seconds" = 30000.
New document → send_keys("^n") before paste."#;

#[cfg(target_os = "macos")]
const FEW_SHOTS: &str = r#"User: When I say open TextEdit, launch TextEdit snapped left
{"kind":"command","confidence":0.9,"summary":"Open TextEdit on the left","command":{"trigger":"open TextEdit","match_mode":"phrase","actions":[{"open_target":{"target":"TextEdit","placement":"left_half"}}]}}

User: Say fetch then open the URL I give you
{"kind":"command","confidence":0.85,"summary":"Fetch URL from speech then open it","command":{"trigger":"fetch","match_mode":"prefix","actions":[{"http_get":{"url":"{{remainder}}"}},{"open_target":{"target":"{{last_result}}"}}]}}

User: Open Chrome and wait two seconds then speak done
{"kind":"command","confidence":0.88,"summary":"Open Chrome, pause, speak","command":{"trigger":"open chrome","match_mode":"phrase","actions":[{"open_target":{"target":"chrome"}},{"wait":{"ms":2000}},{"speak":{"text":"done"}}]}}

User: When I say notes, open TextEdit, create a new note, and paste hello world
{"kind":"command","confidence":0.9,"summary":"Open TextEdit, new note, paste hello world","command":{"trigger":"notes","match_mode":"phrase","actions":[{"open_target":{"target":"TextEdit"}},{"send_keys":{"keys":"^n"}},{"set_clipboard":{"text":"hello world"}},{"send_keys":{"keys":"^v"}}]}}

User: When I say open TextEdit then open TextEdit
{"kind":"command","confidence":0.9,"summary":"Open TextEdit","command":{"trigger":"open TextEdit","match_mode":"phrase","actions":[{"open_target":{"target":"TextEdit"}}]}}

User: When I say TextEdit, open TextEdit fullscreen, new note, paste Hello World, 30 second timer
{"kind":"command","confidence":0.9,"summary":"Open TextEdit fullscreen, new note, paste Hello World, wait 30s","command":{"trigger":"TextEdit","match_mode":"phrase","actions":[{"open_target":{"target":"TextEdit","placement":"maximize"}},{"send_keys":{"keys":"^n"}},{"set_clipboard":{"text":"Hello World"}},{"send_keys":{"keys":"^v"}},{"wait":{"ms":30000}}]}}"#;

#[cfg(not(target_os = "macos"))]
const FEW_SHOTS: &str = r#"User: When I say open notepad, launch Notepad snapped left
{"kind":"command","confidence":0.9,"summary":"Open Notepad on the left","command":{"trigger":"open notepad","match_mode":"phrase","actions":[{"open_target":{"target":"notepad","placement":"left_half"}}]}}

User: Say fetch then open the URL I give you
{"kind":"command","confidence":0.85,"summary":"Fetch URL from speech then open it","command":{"trigger":"fetch","match_mode":"prefix","actions":[{"http_get":{"url":"{{remainder}}"}},{"open_target":{"target":"{{last_result}}"}}]}}

User: Open Chrome and wait two seconds then speak done
{"kind":"command","confidence":0.88,"summary":"Open Chrome, pause, speak","command":{"trigger":"open chrome","match_mode":"phrase","actions":[{"open_target":{"target":"chrome"}},{"wait":{"ms":2000}},{"speak":{"text":"done"}}]}}

User: When I say notepad, open Notepad fullscreen and paste Hello World
{"kind":"command","confidence":0.9,"summary":"Open Notepad fullscreen and paste Hello World","command":{"trigger":"notepad","match_mode":"phrase","actions":[{"open_target":{"target":"notepad","placement":"maximize"}},{"set_clipboard":{"text":"Hello World"}},{"send_keys":{"keys":"^v"}}]}}

User: When I say open notepad then open notepad
{"kind":"command","confidence":0.9,"summary":"Open Notepad","command":{"trigger":"open notepad","match_mode":"phrase","actions":[{"open_target":{"target":"notepad"}}]}}

User: When I say notepad, open notepad fullscreen, new note, paste Hello World, 30 second timer
{"kind":"command","confidence":0.9,"summary":"Open Notepad fullscreen, new note, paste Hello World, wait 30s","command":{"trigger":"notepad","match_mode":"phrase","actions":[{"open_target":{"target":"notepad","placement":"maximize"}},{"send_keys":{"keys":"^n"}},{"set_clipboard":{"text":"Hello World"}},{"send_keys":{"keys":"^v"}},{"wait":{"ms":30000}}]}}"#;

const OUTPUT_SCHEMA: &str = r#"Return ONE JSON object only (no markdown):
{"kind":"command","confidence":0.0-1.0,"summary":"one line","command":{"trigger":"voice phrase","match_mode":"phrase|prefix","actions":[/* Action objects */]}}"#;

/// Build the user-facing composer prompt for a natural-language description.
pub fn build_composer_prompt(description: &str, trigger: Option<&str>) -> String {
    let user_block = match trigger.map(str::trim).filter(|t| !t.is_empty()) {
        Some(phrase) => format!(
            "Trigger phrase (put exactly in JSON \"trigger\" field): \"{phrase}\"\n\
             Actions to perform: {}",
            description.trim()
        ),
        None => description.trim().to_string(),
    };
    format!(
        "You compose voice automations as JSON.\n\n\
         {PLATFORM_RULES}\n\n\
         Actions:\n{ACTION_CATALOG}\n\n\
         {TEMPLATE_RULES}\n\n\
         Examples:\n{FEW_SHOTS}\n\n\
         {OUTPUT_SCHEMA}\n\n\
         User: {user_block}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_includes_catalog_and_few_shots() {
        let prompt = build_composer_prompt("open notepad", None);
        assert!(prompt.contains("open_target"));
        assert!(prompt.contains(r#""kind":"command""#));
        assert!(prompt.contains("{{remainder}}"));
        #[cfg(target_os = "macos")]
        assert!(prompt.contains("TextEdit"));
        #[cfg(not(target_os = "macos"))]
        assert!(prompt.contains("open notepad"));
    }

    #[test]
    fn prompt_includes_platform_rules() {
        let prompt = build_composer_prompt("test", None);
        #[cfg(target_os = "macos")]
        {
            assert!(prompt.contains("Platform: macOS"));
            assert!(prompt.contains("TextEdit"));
            assert!(!prompt.contains("Platform: Windows"));
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert!(prompt.contains("Platform: Windows"));
            assert!(prompt.contains("Notepad"));
        }
    }

    #[test]
    fn prompt_includes_explicit_trigger_block() {
        let prompt = build_composer_prompt("launch Notepad snapped left", Some("open notepad"));
        assert!(prompt.contains(
            r#"Trigger phrase (put exactly in JSON "trigger" field): "open notepad""#
        ));
        assert!(prompt.contains("Actions to perform: launch Notepad snapped left"));
    }

    #[test]
    fn prompt_includes_template_rules_and_new_few_shots() {
        let prompt = build_composer_prompt("test", None);

        assert!(prompt.contains("Never duplicate the trigger as an action"));
        assert!(prompt.contains(r#""then" and "and then" separate steps"#));
        assert!(prompt.contains(r#""30 seconds" = 30000"#));
        assert!(prompt.contains(r#"New document → send_keys("^n") before paste"#));
        assert!(prompt.contains(r#"{"send_keys":{"keys":"^n"}}"#));
        assert!(prompt.contains(r#"{"wait":{"ms":30000}}"#));
        assert!(prompt.contains(r#""trigger":"voice phrase""#));
        assert!(prompt.contains("Put the exact voice activation phrase in \"trigger\""));

        #[cfg(target_os = "macos")]
        {
            assert!(prompt.contains("create a new note, and paste hello world"));
            assert!(prompt.contains(r#""target":"TextEdit""#));
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert!(prompt.contains("When I say open notepad then open notepad"));
            assert!(prompt.contains(
                r#""trigger":"open notepad","match_mode":"phrase","actions":[{"open_target":{"target":"notepad"}}]"#
            ));
            assert!(prompt.contains("30 second timer"));
        }
    }
}

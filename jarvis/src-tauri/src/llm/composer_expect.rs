//! Assertion helpers for composer eval fixtures.

use crate::db::{Action, MatchMode, NewCommandNode};
use crate::llm::composer::{ComposerKind, GenerateAutomationResult};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ComposerExpectation {
    pub kind: ComposerKind,
    #[serde(default)]
    pub trigger_phrases: Vec<String>,
    #[serde(default)]
    pub match_mode: Option<MatchMode>,
    #[serde(default)]
    pub action_checks: Vec<ActionCheck>,
    #[serde(default)]
    pub max_actions: Option<usize>,
    #[serde(default)]
    pub forbid_duplicate_actions: bool,
    #[serde(default)]
    pub warnings_contain: Vec<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_display_name: Option<String>,
    #[serde(default)]
    pub command_absent: bool,
    #[serde(default)]
    pub tool_absent: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ActionCheck {
    pub kind: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub keys: Option<String>,
    #[serde(default)]
    pub ms: Option<u64>,
    #[serde(default)]
    pub placement: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

pub fn assert_result_matches(
    result: &GenerateAutomationResult,
    expect: &ComposerExpectation,
) -> Result<(), String> {
    if result.kind != expect.kind {
        return Err(format!(
            "kind: expected {:?}, got {:?}",
            expect.kind, result.kind
        ));
    }

    if expect.command_absent && result.command.is_some() {
        return Err("expected no command draft".into());
    }
    if expect.tool_absent && result.tool.is_some() {
        return Err("expected no tool draft".into());
    }

    if let Some(name) = &expect.tool_name {
        let tool = result
            .tool
            .as_ref()
            .ok_or_else(|| "expected tool draft".to_string())?;
        if tool.name != *name {
            return Err(format!("tool.name: expected `{name}`, got `{}`", tool.name));
        }
    }

    if let Some(display) = &expect.tool_display_name {
        let tool = result
            .tool
            .as_ref()
            .ok_or_else(|| "expected tool draft".to_string())?;
        if tool.display_name != *display {
            return Err(format!(
                "tool.display_name: expected `{display}`, got `{}`",
                tool.display_name
            ));
        }
    }

    if !expect.trigger_phrases.is_empty()
        || !expect.action_checks.is_empty()
        || expect.match_mode.is_some()
        || expect.max_actions.is_some()
        || expect.forbid_duplicate_actions
    {
        let command = result
            .command
            .as_ref()
            .ok_or_else(|| "expected command draft".to_string())?;
        assert_command_matches(command, expect)?;
    }

    for needle in &expect.warnings_contain {
        if !result
            .warnings
            .iter()
            .any(|w| w.to_ascii_lowercase().contains(&needle.to_ascii_lowercase()))
        {
            return Err(format!(
                "warnings: expected substring `{needle}`, got {:?}",
                result.warnings
            ));
        }
    }

    Ok(())
}

fn assert_command_matches(
    command: &NewCommandNode,
    expect: &ComposerExpectation,
) -> Result<(), String> {
    if !expect.trigger_phrases.is_empty() && command.trigger_phrases != expect.trigger_phrases {
        return Err(format!(
            "trigger_phrases: expected {:?}, got {:?}",
            expect.trigger_phrases, command.trigger_phrases
        ));
    }

    if let Some(mode) = &expect.match_mode {
        if &command.match_mode != mode {
            return Err(format!(
                "match_mode: expected {:?}, got {:?}",
                mode, command.match_mode
            ));
        }
    }

    if let Some(max) = expect.max_actions {
        if command.actions.len() > max {
            return Err(format!(
                "actions.len(): expected at most {max}, got {}",
                command.actions.len()
            ));
        }
    }

    if !expect.action_checks.is_empty() {
        if command.actions.len() < expect.action_checks.len() {
            return Err(format!(
                "actions.len(): expected at least {}, got {}",
                expect.action_checks.len(),
                command.actions.len()
            ));
        }
        for (index, check) in expect.action_checks.iter().enumerate() {
            if !action_matches_check(&command.actions[index], check) {
                return Err(format!(
                    "action {index}: expected {:?}, got {:?}",
                    check,
                    command.actions[index]
                ));
            }
        }
    }

    if expect.forbid_duplicate_actions && has_duplicate_actions(&command.actions) {
        return Err(format!(
            "actions contain duplicates: {:?}",
            command.actions
        ));
    }

    Ok(())
}

pub fn has_duplicate_actions(actions: &[Action]) -> bool {
    actions
        .iter()
        .enumerate()
        .any(|(i, action)| actions[i + 1..].contains(action))
}

fn action_matches_check(action: &Action, check: &ActionCheck) -> bool {
    if action_kind(action) != check.kind {
        return false;
    }
    match action {
        Action::OpenTarget { target, placement } => {
            field_matches(check.target.as_deref(), Some(target.as_str()))
                && field_matches_opt(check.placement.as_deref(), placement.as_deref())
        }
        Action::OpenApp { name, placement, .. } => {
            field_matches(check.target.as_deref(), Some(name.as_str()))
                && field_matches_opt(check.placement.as_deref(), placement.as_deref())
        }
        Action::SendKeys { keys } => field_matches(check.keys.as_deref(), Some(keys.as_str())),
        Action::Wait { ms } => check.ms.is_none_or(|expected| *ms == expected),
        Action::Speak { text } => field_matches(check.text.as_deref(), Some(text.as_str())),
        Action::SetClipboard { text } => {
            field_matches(check.text.as_deref(), Some(text.as_str()))
        }
        Action::HttpGet { url } => field_matches(check.url.as_deref(), Some(url.as_str())),
        Action::GetClipboard {} => check.kind == "get_clipboard",
        _ => true,
    }
}

fn action_kind(action: &Action) -> &'static str {
    match action {
        Action::OpenApp { .. } => "open_app",
        Action::OpenUrl { .. } => "open_url",
        Action::OpenTarget { .. } => "open_target",
        Action::PlaceWindow { .. } => "place_window",
        Action::RunScript { .. } => "run_script",
        Action::RunRegisteredScript { .. } => "run_registered_script",
        Action::SendKeys { .. } => "send_keys",
        Action::Wait { .. } => "wait",
        Action::Speak { .. } => "speak",
        Action::SubPrompt { .. } => "sub_prompt",
        Action::RunCommand { .. } => "run_command",
        Action::ReadFile { .. } => "read_file",
        Action::HttpGet { .. } => "http_get",
        Action::GetClipboard {} => "get_clipboard",
        Action::ShowNotification { .. } => "show_notification",
        Action::TextTrim { .. } => "text_trim",
        Action::TextMatch { .. } => "text_match",
        Action::TextSplit { .. } => "text_split",
        Action::TextCombine { .. } => "text_combine",
        Action::SetClipboard { .. } => "set_clipboard",
        Action::ListFolder { .. } => "list_folder",
        Action::WriteFile { .. } => "write_file",
        Action::GetFileMetadata { .. } => "get_file_metadata",
        Action::Screenshot { .. } => "screenshot",
        Action::DeviceInfo {} => "device_info",
        Action::IfElse { .. } => "if_else",
        Action::StartDictation {} => "start_dictation",
        Action::StopDictation {} => "stop_dictation",
    }
}

fn field_matches(expected: Option<&str>, actual: Option<&str>) -> bool {
    expected.is_none_or(|value| actual == Some(value))
}

fn field_matches_opt(expected: Option<&str>, actual: Option<&str>) -> bool {
    expected.is_none_or(|value| actual == Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::composer::ComposerKind;

    #[test]
    fn has_duplicate_actions_detects_repeated_open_target() {
        let actions = vec![
            Action::OpenTarget {
                target: "notepad".into(),
                placement: Some("left_half".into()),
            },
            Action::OpenTarget {
                target: "notepad".into(),
                placement: Some("left_half".into()),
            },
        ];
        assert!(has_duplicate_actions(&actions));
    }

    #[test]
    fn assert_result_matches_checks_trigger_and_actions() {
        let result = GenerateAutomationResult {
            kind: ComposerKind::Command,
            command: Some(NewCommandNode {
                name: "open notepad".into(),
                trigger_phrases: vec!["open notepad".into()],
                actions: vec![Action::OpenTarget {
                    target: "notepad".into(),
                    placement: None,
                }],
                enabled: true,
                fuzzy_threshold_pct: 0,
                match_mode: MatchMode::Phrase,
            }),
            tool: None,
            confidence: 0.9,
            summary: "Open Notepad".into(),
            warnings: vec![],
        };
        let expect = ComposerExpectation {
            kind: ComposerKind::Command,
            trigger_phrases: vec!["open notepad".into()],
            match_mode: Some(MatchMode::Phrase),
            action_checks: vec![ActionCheck {
                kind: "open_target".into(),
                target: Some("notepad".into()),
                keys: None,
                ms: None,
                placement: None,
                text: None,
                url: None,
            }],
            max_actions: None,
            forbid_duplicate_actions: false,
            warnings_contain: vec![],
            tool_name: None,
            tool_display_name: None,
            command_absent: false,
            tool_absent: false,
        };
        assert_result_matches(&result, &expect).expect("match");
    }
}

//! Deterministic validation for LLM composer command drafts.

use crate::apps::resolve_target::normalize_placement_zone;
use crate::db::{Action, MatchMode, NewCommandNode, NewToolDefinition, ToolParameter};
use serde::Deserialize;
use std::collections::HashSet;

pub const MAX_COMPOSER_ACTIONS: usize = 8;
pub const MAX_IF_ELSE_DEPTH: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RawComposerCommand {
    /// Primary voice activation phrase from the model (preferred over trigger_phrases).
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub trigger_phrases: Vec<String>,
    #[serde(default)]
    pub match_mode: MatchMode,
    #[serde(default)]
    pub actions: Vec<Action>,
}

/// Merge explicit `trigger` with optional alias phrases into the stored trigger list.
pub fn resolve_trigger_phrases(trigger: Option<String>, phrases: Vec<String>) -> Vec<String> {
    let primary = trigger
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let extras: Vec<String> = phrases
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();

    if let Some(primary) = primary {
        let mut out = vec![primary.clone()];
        for phrase in extras {
            if !out
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&phrase))
            {
                out.push(phrase);
            }
        }
        out
    } else {
        extras
    }
}

/// Validate and normalize a command draft from composer JSON.
pub fn validate_command_draft(raw: RawComposerCommand) -> Result<NewCommandNode, Vec<String>> {
    let mut errors = Vec::new();

    let trigger_phrases = resolve_trigger_phrases(raw.trigger, raw.trigger_phrases);
    if trigger_phrases.is_empty() {
        errors.push("trigger (or trigger_phrases) is required".into());
    }

    if raw.actions.is_empty() {
        errors.push("at least one action is required".into());
    }
    if raw.actions.len() > MAX_COMPOSER_ACTIONS {
        errors.push(format!(
            "at most {MAX_COMPOSER_ACTIONS} top-level actions allowed"
        ));
    }
    let if_else_depth = if_else_nesting_depth(&raw.actions);
    if if_else_depth > MAX_IF_ELSE_DEPTH {
        errors.push(format!(
            "if_else nesting depth {if_else_depth} exceeds max {MAX_IF_ELSE_DEPTH}"
        ));
    }

    let mut actions = Vec::with_capacity(raw.actions.len());
    for (index, action) in raw.actions.into_iter().enumerate() {
        match validate_and_normalize_action(action) {
            Ok(normalized) => actions.push(normalized),
            Err(msg) => errors.push(format!("action {index}: {msg}")),
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let name = trigger_phrases
        .first()
        .map(|p| p.chars().take(72).collect())
        .unwrap_or_default();

    Ok(NewCommandNode {
        name,
        trigger_phrases,
        actions,
        enabled: true,
        fuzzy_threshold_pct: 0,
        match_mode: raw.match_mode,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RawComposerTool {
    pub name: String,
    pub display_name: String,
    pub description: String,
    #[serde(default)]
    pub parameters: Vec<ToolParameter>,
    #[serde(default)]
    pub actions: Vec<Action>,
}

/// Validate and normalize a tool draft from composer JSON.
pub fn validate_tool_draft(
    raw: RawComposerTool,
    builtin_names: &[&str],
) -> Result<NewToolDefinition, Vec<String>> {
    validate_tool_draft_inner(raw, builtin_names, false)
}

pub fn validate_tool_draft_allowing_builtin_name(
    raw: RawComposerTool,
    builtin_names: &[&str],
) -> Result<NewToolDefinition, Vec<String>> {
    validate_tool_draft_inner(raw, builtin_names, true)
}

fn validate_tool_draft_inner(
    raw: RawComposerTool,
    builtin_names: &[&str],
    allow_builtin_name: bool,
) -> Result<NewToolDefinition, Vec<String>> {
    let mut errors = Vec::new();

    let name = raw.name.trim().to_string();
    if name.is_empty() {
        errors.push("tool name is required".into());
    } else if !is_snake_case_tool_name(&name) {
        errors.push("tool name must be unique snake_case".into());
    } else if !allow_builtin_name && builtin_names.iter().any(|b| *b == name) {
        errors.push(format!("tool name `{name}` collides with a builtin tool"));
    }

    if raw.display_name.trim().is_empty() {
        errors.push("display_name is required".into());
    }

    if raw.actions.is_empty() {
        errors.push("at least one action is required".into());
    }
    if raw.actions.len() > MAX_COMPOSER_ACTIONS {
        errors.push(format!(
            "at most {MAX_COMPOSER_ACTIONS} top-level actions allowed"
        ));
    }
    let if_else_depth = if_else_nesting_depth(&raw.actions);
    if if_else_depth > MAX_IF_ELSE_DEPTH {
        errors.push(format!(
            "if_else nesting depth {if_else_depth} exceeds max {MAX_IF_ELSE_DEPTH}"
        ));
    }

    let mut actions = Vec::with_capacity(raw.actions.len());
    for (index, action) in raw.actions.into_iter().enumerate() {
        match validate_and_normalize_action(action) {
            Ok(normalized) => actions.push(normalized),
            Err(msg) => errors.push(format!("action {index}: {msg}")),
        }
    }

    let mut seen_params = HashSet::new();
    for param in &raw.parameters {
        let key = param.name.trim();
        if key.is_empty() {
            errors.push("parameter name is required".into());
        } else if !seen_params.insert(key.to_string()) {
            errors.push(format!("duplicate parameter name `{key}`"));
        }
    }

    if errors.is_empty() {
        let used = collect_tool_param_placeholders(&actions);
        let declared: HashSet<String> = raw
            .parameters
            .iter()
            .map(|p| p.name.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        for param in &declared {
            if !used.contains(param) {
                errors.push(format!(
                    "parameter `{param}` is declared but not used in actions"
                ));
            }
        }
        for placeholder in &used {
            if !declared.contains(placeholder) {
                errors.push(format!(
                    "action placeholder `{{{{{placeholder}}}}}` has no matching parameter"
                ));
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(NewToolDefinition {
        name,
        display_name: raw.display_name.trim().to_string(),
        description: raw.description.trim().to_string(),
        parameters: raw.parameters,
        actions,
        enabled: true,
        builtin: false,
    })
}

pub fn is_snake_case_tool_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Collect `{{param}}` placeholders used in tool actions (excludes magic runtime vars).
pub fn collect_tool_param_placeholders(actions: &[Action]) -> HashSet<String> {
    let json = serde_json::to_string(actions).unwrap_or_default();
    let mut found = HashSet::new();
    let bytes = json.as_bytes();
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some((end, name)) = parse_template_name(&json[i + 2..]) {
                if !is_magic_template_var(&name) {
                    found.insert(name);
                }
                i += 2 + end;
                continue;
            }
        }
        i += 1;
    }
    found
}

fn parse_template_name(rest: &str) -> Option<(usize, String)> {
    let end = rest.find("}}")?;
    let name = rest[..end].trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((end + 2, name.to_string()))
}

fn is_magic_template_var(name: &str) -> bool {
    matches!(
        name,
        "remainder" | "last_result" | "follow_up" | "shortcut_input"
    ) || name
        .strip_prefix("step_")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

fn if_else_nesting_depth(actions: &[Action]) -> usize {
    let mut max_depth = 0usize;
    for action in actions {
        if let Action::IfElse {
            then_actions,
            else_actions,
            ..
        } = action
        {
            let nested = 1
                + if_else_nesting_depth(then_actions)
                    .max(if_else_nesting_depth(else_actions));
            max_depth = max_depth.max(nested);
        }
    }
    max_depth
}

fn validate_and_normalize_action(action: Action) -> Result<Action, String> {
    let normalized = normalize_action(action);
    validate_action_fields(&normalized)?;
    if let Action::IfElse {
        then_actions,
        else_actions,
        ..
    } = &normalized
    {
        for (branch, label) in [(&then_actions[..], "then"), (&else_actions[..], "else")] {
            for (i, child) in branch.iter().enumerate() {
                validate_action_fields(child)
                    .map_err(|e| format!("if_else {label} branch action {i}: {e}"))?;
            }
        }
    }
    Ok(normalized)
}

fn normalize_placement_opt(placement: Option<String>) -> Option<String> {
    placement
        .as_ref()
        .and_then(|p| normalize_placement_zone(p))
        .filter(|p| !p.is_empty())
}

fn normalize_action(action: Action) -> Action {
    match action {
        Action::OpenApp {
            name,
            path,
            placement,
        } if path.trim().is_empty() => Action::OpenTarget {
            target: if name.trim().is_empty() {
                path
            } else {
                name
            },
            placement: normalize_placement_opt(placement),
        },
        Action::OpenApp {
            name,
            path,
            placement,
        } => Action::OpenApp {
            name,
            path,
            placement: normalize_placement_opt(placement),
        },
        Action::OpenTarget { target, placement } => Action::OpenTarget {
            target,
            placement: normalize_placement_opt(placement),
        },
        Action::PlaceWindow { zone, monitor } => Action::PlaceWindow {
            zone: normalize_placement_zone(&zone).unwrap_or(zone),
            monitor,
        },
        Action::OpenUrl { url } => {
            if let Some(target) = maybe_rewrite_guessed_url(&url) {
                Action::OpenTarget {
                    target,
                    placement: None,
                }
            } else {
                Action::OpenUrl { url }
            }
        }
        Action::IfElse {
            condition,
            text,
            pattern,
            then_actions,
            else_actions,
        } => Action::IfElse {
            condition,
            text,
            pattern,
            then_actions: then_actions
                .into_iter()
                .map(normalize_action)
                .collect(),
            else_actions: else_actions
                .into_iter()
                .map(normalize_action)
                .collect(),
        },
        other => other,
    }
}

fn maybe_rewrite_guessed_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let parsed = url::Url::parse(trimmed).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?;
    let label = host.strip_suffix(".com")?;
    if label.contains('.') || label.is_empty() {
        return None;
    }
    Some(label.to_string())
}

fn validate_action_fields(action: &Action) -> Result<(), String> {
    match action {
        Action::OpenUrl { url } => validate_http_url(url, "open_url"),
        Action::HttpGet { url } => validate_http_url(url, "http_get"),
        Action::RunCommand { command_id, .. } if *command_id == 0 => {
            Err("run_command requires a non-zero command_id".into())
        }
        Action::ReadFile { path }
        | Action::ListFolder { path }
        | Action::GetFileMetadata { path }
        | Action::WriteFile { path, .. } => {
            if path.trim().is_empty() {
                Err("path is required".into())
            } else {
                Ok(())
            }
        }
        Action::ShowNotification { title, body } => {
            if title.trim().is_empty() && body.trim().is_empty() {
                Err("notification needs a title or body".into())
            } else {
                Ok(())
            }
        }
        Action::TextMatch { pattern, .. } => {
            if pattern.trim().is_empty() {
                Err("regex pattern is required".into())
            } else {
                Ok(())
            }
        }
        Action::TextSplit { delimiter, .. } => {
            if delimiter.is_empty() {
                Err("delimiter is required".into())
            } else {
                Ok(())
            }
        }
        Action::SubPrompt { prompt } => {
            if prompt.trim().is_empty() {
                Err("follow-up prompt text is required".into())
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

fn validate_http_url(url: &str, kind: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(format!("{kind} url is required"));
    }
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.contains("{{")
    {
        Ok(())
    } else {
        Err(format!("{kind} url must use http:// or https://"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Action, ToolParameter};

    #[test]
    fn validate_normalizes_fullscreen_placement() {
        let raw = RawComposerCommand {
            trigger: None,
            trigger_phrases: vec!["notepad".into()],
            match_mode: MatchMode::Phrase,
            actions: vec![Action::OpenTarget {
                target: "notepad".into(),
                placement: Some("fullscreen".into()),
            }],
        };
        let draft = validate_command_draft(raw).expect("valid command");
        assert!(matches!(
            &draft.actions[0],
            Action::OpenTarget {
                placement: Some(zone),
                ..
            } if zone == "maximize"
        ));
    }

    #[test]
    fn validate_accepts_open_target_command() {
        let raw = RawComposerCommand {
            trigger: None,
            trigger_phrases: vec!["open notepad".into()],
            match_mode: MatchMode::Phrase,
            actions: vec![Action::OpenTarget {
                target: "notepad".into(),
                placement: Some("left_half".into()),
            }],
        };
        let draft = validate_command_draft(raw).expect("valid command");
        assert_eq!(draft.trigger_phrases, vec!["open notepad"]);
        assert!(matches!(
            &draft.actions[0],
            Action::OpenTarget {
                target,
                placement: Some(_)
            } if target == "notepad"
        ));
    }

    #[test]
    fn validate_rejects_empty_triggers() {
        let raw = RawComposerCommand {
            trigger: None,
            trigger_phrases: vec![],
            match_mode: MatchMode::Phrase,
            actions: vec![Action::Speak {
                text: "hi".into(),
            }],
        };
        let err = validate_command_draft(raw).unwrap_err();
        assert!(err.iter().any(|e| e.contains("trigger")));
    }

    #[test]
    fn validate_accepts_trigger_field_without_trigger_phrases() {
        let raw = RawComposerCommand {
            trigger: Some("open notepad".into()),
            trigger_phrases: vec![],
            match_mode: MatchMode::Phrase,
            actions: vec![Action::OpenTarget {
                target: "notepad".into(),
                placement: None,
            }],
        };
        let draft = validate_command_draft(raw).expect("valid command");
        assert_eq!(draft.trigger_phrases, vec!["open notepad"]);
    }

    #[test]
    fn resolve_trigger_prefers_explicit_trigger_over_phrases() {
        let phrases = resolve_trigger_phrases(
            Some("notepad".into()),
            vec!["open notepad then open notepad".into(), "pad".into()],
        );
        assert_eq!(phrases, vec!["notepad", "open notepad then open notepad", "pad"]);
    }

    #[test]
    fn resolve_trigger_dedupes_case_insensitive_aliases() {
        let phrases = resolve_trigger_phrases(
            Some("Fetch".into()),
            vec!["fetch".into(), "get url".into()],
        );
        assert_eq!(phrases, vec!["Fetch", "get url"]);
    }

    #[test]
    fn validate_rejects_chain_over_limit() {
        let actions: Vec<Action> = (0..9)
            .map(|_| Action::Wait { ms: 1 })
            .collect();
        let raw = RawComposerCommand {
            trigger: None,
            trigger_phrases: vec!["test".into()],
            match_mode: MatchMode::Phrase,
            actions,
        };
        let err = validate_command_draft(raw).unwrap_err();
        assert!(err.iter().any(|e| e.contains("8")));
    }

    #[test]
    fn normalize_open_app_empty_path_to_open_target() {
        let raw = RawComposerCommand {
            trigger: None,
            trigger_phrases: vec!["launch".into()],
            match_mode: MatchMode::Phrase,
            actions: vec![Action::OpenApp {
                name: "notepad".into(),
                path: String::new(),
                placement: None,
            }],
        };
        let draft = validate_command_draft(raw).unwrap();
        assert!(matches!(
            &draft.actions[0],
            Action::OpenTarget { target, .. } if target == "notepad"
        ));
    }

    #[test]
    fn normalize_guessed_url_to_open_target() {
        let raw = RawComposerCommand {
            trigger: None,
            trigger_phrases: vec!["open site".into()],
            match_mode: MatchMode::Phrase,
            actions: vec![Action::OpenUrl {
                url: "https://notepad.com".into(),
            }],
        };
        let draft = validate_command_draft(raw).unwrap();
        assert!(matches!(
            &draft.actions[0],
            Action::OpenTarget { target, .. } if target == "notepad"
        ));
    }

    #[test]
    fn validate_rejects_non_http_open_url() {
        let raw = RawComposerCommand {
            trigger: None,
            trigger_phrases: vec!["open".into()],
            match_mode: MatchMode::Phrase,
            actions: vec![Action::OpenUrl {
                url: "ftp://example.com".into(),
            }],
        };
        let err = validate_command_draft(raw).unwrap_err();
        assert!(err.iter().any(|e| e.contains("http://")));
    }

    #[test]
    fn validate_rejects_run_command_zero_id() {
        let raw = RawComposerCommand {
            trigger: None,
            trigger_phrases: vec!["run".into()],
            match_mode: MatchMode::Phrase,
            actions: vec![Action::RunCommand {
                command_id: 0,
                input: None,
            }],
        };
        let err = validate_command_draft(raw).unwrap_err();
        assert!(err.iter().any(|e| e.contains("command_id")));
    }

    #[test]
    fn validate_accepts_speak_tool() {
        let raw = RawComposerTool {
            name: "speak_text".into(),
            display_name: "Speak text".into(),
            description: "Speak arbitrary text".into(),
            parameters: vec![ToolParameter {
                name: "text".into(),
                param_type: "string".into(),
                description: Some("Text to speak".into()),
                required: true,
                enum_values: vec![],
            }],
            actions: vec![Action::Speak {
                text: "{{text}}".into(),
            }],
        };
        let draft = validate_tool_draft(raw, &["open_url"]).expect("valid tool");
        assert_eq!(draft.name, "speak_text");
        assert!(!draft.builtin);
        assert!(matches!(&draft.actions[0], Action::Speak { text } if text == "{{text}}"));
    }

    #[test]
    fn validate_rejects_non_snake_case_tool_name() {
        let raw = RawComposerTool {
            name: "SpeakText".into(),
            display_name: "Speak".into(),
            description: "desc".into(),
            parameters: vec![],
            actions: vec![Action::Speak {
                text: "hi".into(),
            }],
        };
        let err = validate_tool_draft(raw, &[]).unwrap_err();
        assert!(err.iter().any(|e| e.contains("snake_case")));
    }

    #[test]
    fn validate_rejects_builtin_tool_name_collision() {
        let raw = RawComposerTool {
            name: "open_url".into(),
            display_name: "Open URL".into(),
            description: "desc".into(),
            parameters: vec![],
            actions: vec![Action::OpenUrl {
                url: "https://example.com".into(),
            }],
        };
        let err = validate_tool_draft(raw, &["open_url"]).unwrap_err();
        assert!(err.iter().any(|e| e.contains("builtin")));
    }

    #[test]
    fn validate_rejects_param_not_used_in_actions() {
        let raw = RawComposerTool {
            name: "speak_text".into(),
            display_name: "Speak".into(),
            description: "desc".into(),
            parameters: vec![ToolParameter {
                name: "text".into(),
                param_type: "string".into(),
                description: None,
                required: true,
                enum_values: vec![],
            }],
            actions: vec![Action::Speak {
                text: "hello".into(),
            }],
        };
        let err = validate_tool_draft(raw, &[]).unwrap_err();
        assert!(err.iter().any(|e| e.contains("not used")));
    }

    #[test]
    fn validate_rejects_placeholder_without_param() {
        let raw = RawComposerTool {
            name: "speak_text".into(),
            display_name: "Speak".into(),
            description: "desc".into(),
            parameters: vec![],
            actions: vec![Action::Speak {
                text: "{{text}}".into(),
            }],
        };
        let err = validate_tool_draft(raw, &[]).unwrap_err();
        assert!(err.iter().any(|e| e.contains("no matching parameter")));
    }

    #[test]
    fn validate_rejects_tool_chain_over_limit() {
        let actions: Vec<Action> = (0..9).map(|_| Action::Wait { ms: 1 }).collect();
        let raw = RawComposerTool {
            name: "many_waits".into(),
            display_name: "Waits".into(),
            description: "desc".into(),
            parameters: vec![],
            actions,
        };
        let err = validate_tool_draft(raw, &[]).unwrap_err();
        assert!(err.iter().any(|e| e.contains("8")));
    }
}

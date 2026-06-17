//! Parse, validate, and orchestrate LLM composer output.

use crate::apps::resolve_target::{normalize_placement_zone, strip_placement_suffix};
use crate::db::{Action, MatchMode, NewCommandNode, NewToolDefinition, ToolParameter};
use crate::llm::composer_prompt::build_composer_prompt;
use crate::llm::composer_validate::{
    collect_tool_param_placeholders, validate_command_draft,
    validate_tool_draft, validate_tool_draft_allowing_builtin_name, RawComposerCommand,
    RawComposerTool,
};
use crate::llm::router::extract_json_object;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const COMPOSER_LOW_CONFIDENCE_THRESHOLD: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposerKind {
    Command,
    Tool,
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateAutomationResult {
    pub kind: ComposerKind,
    pub command: Option<NewCommandNode>,
    pub tool: Option<NewToolDefinition>,
    pub confidence: f32,
    pub summary: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposerErrorCode {
    FeatureDisabled,
    ModelMissing,
    ModelLoading,
    InvalidJson,
    SchemaInvalid,
    InferFailed,
    UnsupportedKind,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComposerError {
    pub code: ComposerErrorCode,
    pub message: String,
}

impl ComposerError {
    pub fn into_string(self) -> String {
        serde_json::to_string(&self).unwrap_or(self.message)
    }
}

/// Abstraction for unit tests (mock) and local GGUF inference.
pub trait ComposerInfer {
    fn infer(&self, prompt: &str) -> Result<String, String>;
}

#[derive(Debug, Deserialize)]
pub struct RawComposerOutput {
    kind: String,
    confidence: f64,
    summary: String,
    command: Option<RawComposerCommand>,
    tool: Option<RawComposerTool>,
}

pub fn parse_composer_output(raw: &str) -> Result<RawComposerOutput, ComposerError> {
    let trimmed = raw.trim();
    let json_text = extract_json_object(trimmed).unwrap_or_else(|| trimmed.to_string());
    serde_json::from_str::<RawComposerOutput>(&json_text).map_err(|e| ComposerError {
        code: ComposerErrorCode::InvalidJson,
        message: format!("composer output is not valid JSON: {e}"),
    })
}

pub fn finalize_automation_result(
    raw: RawComposerOutput,
    user_description: &str,
    builtin_tool_names: &[&str],
) -> Result<GenerateAutomationResult, ComposerError> {
    let mut kind = parse_kind(&raw.kind)?;
    let mut command = match kind {
        ComposerKind::Command | ComposerKind::Both => {
            let command_raw = raw.command.ok_or_else(|| ComposerError {
                code: ComposerErrorCode::SchemaInvalid,
                message: "command field is required for command/both kinds".into(),
            })?;
            Some(validate_command_draft(command_raw).map_err(|errors| ComposerError {
                code: ComposerErrorCode::SchemaInvalid,
                message: errors.join("; "),
            })?)
        }
        ComposerKind::Tool => None,
    };
    let mut tool = match kind {
        ComposerKind::Tool | ComposerKind::Both => {
            let tool_raw = raw.tool.ok_or_else(|| ComposerError {
                code: ComposerErrorCode::SchemaInvalid,
                message: "tool field is required for tool/both kinds".into(),
            })?;
            Some(
                if kind == ComposerKind::Both {
                    validate_tool_draft_allowing_builtin_name(tool_raw, builtin_tool_names)
                } else {
                    validate_tool_draft(tool_raw, builtin_tool_names)
                }
                .map_err(|errors| ComposerError {
                    code: ComposerErrorCode::SchemaInvalid,
                    message: errors.join("; "),
                })?,
            )
        }
        ComposerKind::Command => None,
    };

    let (split_kind, split_warnings) =
        smart_split(kind, &mut command, &mut tool, user_description, builtin_tool_names);
    kind = split_kind;

    let mut warnings = split_warnings;

    if let Some(cmd) = command.as_mut() {
        apply_description_placement_to_command(cmd, user_description);
        warnings.extend(align_trigger_from_description(cmd, user_description));
        warnings.extend(dedupe_consecutive_actions(&mut cmd.actions));
        warnings.extend(composer_semantic_check(user_description, cmd));
    }

    let confidence = raw.confidence.clamp(0.0, 1.0) as f32;
    if confidence < COMPOSER_LOW_CONFIDENCE_THRESHOLD {
        warnings.push(format!(
            "Low confidence ({confidence:.0}%) — review the draft before saving."
        ));
    }

    Ok(GenerateAutomationResult {
        kind,
        command,
        tool,
        confidence,
        summary: raw.summary.trim().to_string(),
        warnings,
    })
}

/// Deterministic overrides after schema validation.
pub fn smart_split(
    kind: ComposerKind,
    command: &mut Option<NewCommandNode>,
    tool: &mut Option<NewToolDefinition>,
    user_description: &str,
    builtin_tool_names: &[&str],
) -> (ComposerKind, Vec<String>) {
    let mut warnings = Vec::new();
    let mut next_kind = kind;
    let user_has_trigger = user_description_has_trigger(user_description);

    if next_kind == ComposerKind::Tool && user_has_trigger {
        if let Some(tool_draft) = tool.as_ref() {
            let trigger = extract_trigger_from_description(user_description)
                .unwrap_or_else(|| tool_draft.display_name.clone());
            let synthesized = synthesize_command_from_tool(tool_draft, &trigger);
            if tool_draft.parameters.is_empty() {
                *command = Some(synthesized);
                *tool = None;
                next_kind = ComposerKind::Command;
            } else {
                *command = Some(synthesized);
                next_kind = ComposerKind::Both;
            }
        }
    }

    if next_kind == ComposerKind::Command {
        if let Some(cmd) = command.as_mut() {
            if command_uses_only_remainder_or_subprompt(cmd) {
                if user_has_trigger && cmd.match_mode != MatchMode::Prefix {
                    cmd.match_mode = MatchMode::Prefix;
                    warnings.push(
                        "Switched to prefix match because actions use speech remainder input."
                            .into(),
                    );
                } else if !user_has_trigger {
                    *tool = Some(command_to_tool(cmd));
                    *command = None;
                    next_kind = ComposerKind::Tool;
                }
            }
        }
    }

    if next_kind == ComposerKind::Both {
        if let Some(tool_draft) = tool.as_mut() {
            if builtin_tool_names.contains(&tool_draft.name.as_str()) {
                let original = tool_draft.name.clone();
                tool_draft.name = uniquify_tool_name(&tool_draft.name, builtin_tool_names);
                warnings.push(format!(
                    "Tool name `{original}` collides with a builtin; renamed to `{}`.",
                    tool_draft.name
                ));
            }
        }
    }

    (next_kind, warnings)
}

/// When the LLM omits `placement`, infer it from phrases like "in fullscreen" in the user text.
fn apply_description_placement_to_command(command: &mut NewCommandNode, description: &str) {
    let (_, suffix_placement) = strip_placement_suffix(description);
    let Some(zone) = suffix_placement.and_then(|z| normalize_placement_zone(&z)) else {
        return;
    };
    for action in &mut command.actions {
        match action {
            Action::OpenTarget { placement, .. } | Action::OpenApp { placement, .. } => {
                if placement.is_none() {
                    *placement = Some(zone.clone());
                }
            }
            _ => {}
        }
    }
}

pub fn user_description_has_trigger(description: &str) -> bool {
    let lower = description.to_ascii_lowercase();
    lower.contains("when i say")
        || lower.contains("when i tell")
        || description.contains('"')
        || description.contains('\'')
}

pub fn extract_trigger_from_description(description: &str) -> Option<String> {
    if let Some(start) = description.find('"') {
        let rest = &description[start + 1..];
        if let Some(end) = rest.find('"') {
            let phrase = trim_trigger_phrase(&rest[..end]);
            if !phrase.is_empty() {
                return Some(phrase);
            }
        }
    }
    if let Some(start) = description.find('\'') {
        let rest = &description[start + 1..];
        if let Some(end) = rest.find('\'') {
            let phrase = trim_trigger_phrase(&rest[..end]);
            if !phrase.is_empty() {
                return Some(phrase);
            }
        }
    }
    let lower = description.to_ascii_lowercase();
    for prefix in ["when i say ", "when i tell "] {
        if let Some(idx) = lower.find(prefix) {
            let rest = description[idx + prefix.len()..].trim();
            let phrase = trim_trigger_phrase(rest);
            if !phrase.is_empty() {
                return Some(phrase);
            }
        }
    }
    None
}

fn trim_trigger_phrase(phrase: &str) -> String {
    let phrase = phrase.trim();
    let lower = phrase.to_ascii_lowercase();
    let mut cut_at = phrase.len();
    for boundary in [" and then ", " after that ", " then "] {
        if let Some(idx) = lower.find(boundary) {
            cut_at = cut_at.min(idx);
        }
    }
    phrase[..cut_at]
        .split([',', '.', ';'])
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn dedupe_consecutive_actions(actions: &mut Vec<Action>) -> Vec<String> {
    let mut warnings = Vec::new();
    if actions.is_empty() {
        return warnings;
    }
    let mut deduped = vec![actions[0].clone()];
    for action in actions.iter().skip(1) {
        let prev_json = serde_json::to_string(deduped.last().unwrap()).unwrap_or_default();
        let curr_json = serde_json::to_string(action).unwrap_or_default();
        if curr_json == prev_json {
            warnings.push("Removed duplicate action.".into());
        } else {
            deduped.push(action.clone());
        }
    }
    *actions = deduped;
    warnings
}

fn align_trigger_from_description(cmd: &mut NewCommandNode, description: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    if !user_description_has_trigger(description) {
        return warnings;
    }
    let Some(extracted) = extract_trigger_from_description(description) else {
        return warnings;
    };
    let current = cmd.trigger_phrases.first().cloned().unwrap_or_default();
    if extracted == current {
        return warnings;
    }
    let llm_contains_then = current.to_ascii_lowercase().contains("then");
    let extracted_shorter = extracted.len() < current.len();
    if llm_contains_then || extracted_shorter {
        if cmd.trigger_phrases.is_empty() {
            cmd.trigger_phrases.push(extracted.clone());
        } else {
            cmd.trigger_phrases[0] = extracted.clone();
        }
        cmd.name = extracted.chars().take(72).collect();
        warnings.push("Adjusted trigger from your description.".into());
    }
    warnings
}

fn composer_semantic_check(description: &str, cmd: &NewCommandNode) -> Vec<String> {
    let mut warnings = Vec::new();
    let lower = description.to_ascii_lowercase();
    let mentions_delay = lower.contains("timer")
        || lower.contains("second")
        || lower.contains("seconds");
    let has_wait = cmd
        .actions
        .iter()
        .any(|action| matches!(action, Action::Wait { .. }));
    if mentions_delay && !has_wait {
        warnings.push(
            "Description mentions a timer or delay, but no Wait action was included.".into(),
        );
    }
    let mentions_new_doc = lower.contains("new note") || lower.contains("new document");
    let has_new_doc_keys = cmd.actions.iter().any(|action| {
        matches!(action, Action::SendKeys { keys } if send_keys_is_new_document(keys))
    });
    if mentions_new_doc && !has_new_doc_keys {
        warnings.push(
            "Description mentions a new note or document, but no Ctrl+N send_keys was included."
                .into(),
        );
    }
    warnings
}

fn send_keys_is_new_document(keys: &str) -> bool {
    let lower = keys.to_ascii_lowercase();
    lower.contains("^n") || lower.contains("ctrl+n")
}

fn synthesize_command_from_tool(tool: &NewToolDefinition, trigger: &str) -> NewCommandNode {
    let match_mode = if tool.parameters.is_empty() {
        MatchMode::Phrase
    } else {
        MatchMode::Prefix
    };
    NewCommandNode {
        name: trigger.chars().take(72).collect(),
        trigger_phrases: vec![trigger.to_string()],
        actions: tool.actions.clone(),
        enabled: true,
        fuzzy_threshold_pct: 0,
        match_mode,
    }
}

fn command_to_tool(command: &NewCommandNode) -> NewToolDefinition {
    let placeholders = collect_tool_param_placeholders(&command.actions);
    let trigger = command
        .trigger_phrases
        .first()
        .cloned()
        .unwrap_or_else(|| command.name.clone());
    let name = trigger_to_snake_case(&trigger);
    NewToolDefinition {
        name,
        display_name: trigger,
        description: format!("Generated tool from composer draft: {}", command.name),
        parameters: placeholders
            .into_iter()
            .map(|name| ToolParameter {
                name: name.clone(),
                param_type: "string".into(),
                description: None,
                required: true,
                enum_values: vec![],
            })
            .collect(),
        actions: command.actions.clone(),
        enabled: true,
        builtin: false,
    }
}

fn trigger_to_snake_case(trigger: &str) -> String {
    let mut out = String::new();
    for (i, word) in trigger
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .enumerate()
    {
        if i > 0 {
            out.push('_');
        }
        for ch in word.chars() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch.to_ascii_lowercase());
            } else if ch == '-' || ch == '_' {
                out.push('_');
            }
        }
    }
    if out.is_empty() {
        "custom_tool".into()
    } else {
        out
    }
}

fn uniquify_tool_name(base: &str, reserved: &[&str]) -> String {
    let candidate = format!("{base}_custom");
    if !reserved.contains(&candidate.as_str()) {
        return candidate;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}_custom_{n}");
        if !reserved.contains(&candidate.as_str()) {
            return candidate;
        }
        n += 1;
    }
}

fn command_uses_only_remainder_or_subprompt(command: &NewCommandNode) -> bool {
    !command.actions.is_empty()
        && command
            .actions
            .iter()
            .all(action_uses_only_remainder_or_subprompt)
}

fn action_uses_only_remainder_or_subprompt(action: &crate::db::Action) -> bool {
    use crate::db::Action;
    match action {
        Action::SubPrompt { .. } => true,
        _ => {
            let json = serde_json::to_string(action).unwrap_or_default();
            let placeholders = collect_placeholders_from_json(&json);
            !placeholders.is_empty()
                && placeholders
                    .iter()
                    .all(|name| is_runtime_template_var(name))
        }
    }
}

fn collect_placeholders_from_json(json: &str) -> HashSet<String> {
    let mut found = HashSet::new();
    let bytes = json.as_bytes();
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end_rel) = json[i + 2..].find("}}") {
                let name = json[i + 2..i + 2 + end_rel].trim();
                if !name.is_empty() {
                    found.insert(name.to_string());
                }
                i += 2 + end_rel + 2;
                continue;
            }
        }
        i += 1;
    }
    found
}

fn is_runtime_template_var(name: &str) -> bool {
    matches!(
        name,
        "remainder" | "last_result" | "follow_up" | "shortcut_input"
    ) || name
        .strip_prefix("step_")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

fn parse_kind(kind: &str) -> Result<ComposerKind, ComposerError> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "command" => Ok(ComposerKind::Command),
        "tool" => Ok(ComposerKind::Tool),
        "both" => Ok(ComposerKind::Both),
        other => Err(ComposerError {
            code: ComposerErrorCode::UnsupportedKind,
            message: format!("unknown kind `{other}`"),
        }),
    }
}

/// Combine optional UI trigger with action description for alignment heuristics.
pub fn compose_user_description(description: &str, trigger: Option<&str>) -> String {
    match trigger.map(str::trim).filter(|t| !t.is_empty()) {
        Some(phrase) => format!("When I say {phrase}, {}", description.trim()),
        None => description.trim().to_string(),
    }
}

/// One-shot repair: re-prompt with validation errors, then parse again.
pub fn generate_automation_with_infer(
    description: &str,
    trigger: Option<&str>,
    infer: &dyn ComposerInfer,
    builtin_tool_names: &[&str],
) -> Result<GenerateAutomationResult, ComposerError> {
    let user_description = compose_user_description(description, trigger);
    let prompt = build_composer_prompt(description, trigger);
    let raw_text = infer.infer(&prompt).map_err(|message| ComposerError {
        code: if message == "composer inference cancelled" {
            ComposerErrorCode::Cancelled
        } else {
            ComposerErrorCode::InferFailed
        },
        message,
    })?;

    let finalize = |raw: RawComposerOutput| {
        finalize_automation_result(raw, &user_description, builtin_tool_names)
    };

    match parse_composer_output(&raw_text).and_then(finalize) {
        Ok(result) => Ok(result),
        Err(first_err) => {
            if !matches!(
                first_err.code,
                ComposerErrorCode::SchemaInvalid | ComposerErrorCode::InvalidJson
            ) {
                return Err(first_err);
            }
            let repair_prompt = format!(
                "{prompt}\n\n\
                 Your previous JSON failed validation: {}\n\
                 Return corrected JSON only.",
                first_err.message
            );
            let repaired = infer.infer(&repair_prompt).map_err(|message| ComposerError {
                code: ComposerErrorCode::InferFailed,
                message,
            })?;
            parse_composer_output(&repaired).and_then(finalize)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Action, MatchMode, NewCommandNode};

    struct MockInfer(&'static str);

    impl ComposerInfer for MockInfer {
        fn infer(&self, _prompt: &str) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }

    const BUILTINS: &[&str] = &["open_url", "open_target", "snap_window"];

    #[test]
    fn apply_description_placement_when_llm_omits_it() {
        let json = r#"{
            "kind": "command",
            "confidence": 0.9,
            "summary": "Open Notepad fullscreen",
            "command": {
                "trigger_phrases": ["notepad"],
                "match_mode": "phrase",
                "actions": [{"open_target": {"target": "notepad"}}]
            }
        }"#;
        let raw = parse_composer_output(json).unwrap();
        let result = finalize_automation_result(
            raw,
            "When I say notepad open notepad in fullscreen",
            BUILTINS,
        )
        .unwrap();
        let cmd = result.command.unwrap();
        assert!(matches!(
            &cmd.actions[0],
            Action::OpenTarget {
                placement: Some(zone),
                ..
            } if zone == "maximize"
        ));
    }

    #[test]
    fn parse_notepad_fullscreen_paste_command_draft() {
        let json = r#"{
            "kind": "command",
            "confidence": 0.9,
            "summary": "Open Notepad fullscreen and paste Hello World",
            "command": {
                "trigger_phrases": ["notepad"],
                "match_mode": "phrase",
                "actions": [
                    {"open_target": {"target": "notepad", "placement": "maximize"}},
                    {"set_clipboard": {"text": "Hello World"}},
                    {"send_keys": {"keys": "^v"}}
                ]
            }
        }"#;
        let raw = parse_composer_output(json).unwrap();
        let result = finalize_automation_result(
            raw,
            r#"When I say notepad, open notepad in fullscreen and paste "Hello World" into a new note."#,
            BUILTINS,
        )
        .unwrap();
        assert_eq!(result.kind, ComposerKind::Command);
        let cmd = result.command.unwrap();
        assert_eq!(cmd.trigger_phrases, vec!["notepad"]);
        assert_eq!(cmd.actions.len(), 3);
    }

    #[test]
    fn parse_open_notepad_command_draft() {
        let json = r#"{
            "kind": "command",
            "confidence": 0.92,
            "summary": "Open Notepad on the left",
            "command": {
                "trigger_phrases": ["open notepad"],
                "match_mode": "phrase",
                "actions": [{"open_target": {"target": "notepad", "placement": "left_half"}}]
            }
        }"#;
        let raw = parse_composer_output(json).unwrap();
        let result =
            finalize_automation_result(raw, "open notepad on the left", BUILTINS).unwrap();
        assert_eq!(result.kind, ComposerKind::Command);
        assert!((result.confidence - 0.92).abs() < 0.001);
        let cmd = result.command.unwrap();
        assert_eq!(cmd.trigger_phrases, vec!["open notepad"]);
        assert!(matches!(
            &cmd.actions[0],
            Action::OpenTarget { target, .. } if target == "notepad"
        ));
    }

    #[test]
    fn parse_speak_tool_draft() {
        let json = r#"{
            "kind": "tool",
            "confidence": 0.85,
            "summary": "Speak arbitrary text",
            "tool": {
                "name": "speak_text",
                "display_name": "Speak text",
                "description": "Speak what the user says",
                "parameters": [{"name":"text","param_type":"string","required":true}],
                "actions": [{"speak": {"text": "{{text}}"}}]
            }
        }"#;
        let raw = parse_composer_output(json).unwrap();
        let result = finalize_automation_result(raw, "speak whatever I type", BUILTINS).unwrap();
        assert_eq!(result.kind, ComposerKind::Tool);
        let tool = result.tool.unwrap();
        assert_eq!(tool.name, "speak_text");
        assert!(result.command.is_none());
    }

    #[test]
    fn smart_split_upgrades_tool_when_user_gave_trigger() {
        let json = r#"{
            "kind": "tool",
            "confidence": 0.8,
            "summary": "Open notepad",
            "tool": {
                "name": "open_notepad",
                "display_name": "Open Notepad",
                "description": "Launch notepad",
                "parameters": [],
                "actions": [{"open_target": {"target": "notepad"}}]
            }
        }"#;
        let raw = parse_composer_output(json).unwrap();
        let result = finalize_automation_result(
            raw,
            "When I say \"open notepad\", launch notepad",
            BUILTINS,
        )
        .unwrap();
        assert_eq!(result.kind, ComposerKind::Command);
        assert!(result.tool.is_none());
        assert_eq!(
            result.command.unwrap().trigger_phrases,
            vec!["open notepad"]
        );
    }

    #[test]
    fn smart_split_downgrades_remainder_command_without_user_trigger() {
        let json = r#"{
            "kind": "command",
            "confidence": 0.75,
            "summary": "Fetch URL",
            "command": {
                "trigger_phrases": ["fetch"],
                "match_mode": "phrase",
                "actions": [{"http_get": {"url": "{{remainder}}"}}]
            }
        }"#;
        let raw = parse_composer_output(json).unwrap();
        let result =
            finalize_automation_result(raw, "fetch a URL for me", BUILTINS).unwrap();
        assert_eq!(result.kind, ComposerKind::Tool);
        assert!(result.command.is_none());
        assert!(result.tool.is_some());
    }

    #[test]
    fn smart_split_suffixes_colliding_tool_name_on_both() {
        let json = r#"{
            "kind": "both",
            "confidence": 0.9,
            "summary": "Fetch and open",
            "command": {
                "trigger_phrases": ["fetch"],
                "match_mode": "prefix",
                "actions": [{"http_get": {"url": "{{remainder}}"}}]
            },
            "tool": {
                "name": "open_url",
                "display_name": "Open URL",
                "description": "Open a URL",
                "parameters": [{"name":"url","param_type":"string","required":true}],
                "actions": [{"open_url": {"url": "{{url}}"}}]
            }
        }"#;
        let raw = parse_composer_output(json).unwrap();
        let result = finalize_automation_result(
            raw,
            "When I say fetch, get the URL then open it",
            BUILTINS,
        )
        .unwrap();
        assert_eq!(result.kind, ComposerKind::Both);
        let tool = result.tool.unwrap();
        assert_eq!(tool.name, "open_url_custom");
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("collides")));
    }

    #[test]
    fn finalize_both_returns_command_and_tool() {
        let json = r#"{
            "kind": "both",
            "confidence": 0.88,
            "summary": "Speak with trigger",
            "command": {
                "trigger_phrases": ["speak"],
                "match_mode": "prefix",
                "actions": [{"speak": {"text": "{{text}}"}}]
            },
            "tool": {
                "name": "speak_custom",
                "display_name": "Speak",
                "description": "Speak text",
                "parameters": [{"name":"text","param_type":"string","required":true}],
                "actions": [{"speak": {"text": "{{text}}"}}]
            }
        }"#;
        let raw = parse_composer_output(json).unwrap();
        let result = finalize_automation_result(raw, "when I say speak", BUILTINS).unwrap();
        assert_eq!(result.kind, ComposerKind::Both);
        assert!(result.command.is_some());
        assert!(result.tool.is_some());
    }

    #[test]
    fn generate_with_mock_infer_uses_trigger_field() {
        let json = r#"{"kind":"command","confidence":0.9,"summary":"Open Notepad","command":{"trigger":"open notepad","match_mode":"phrase","actions":[{"open_target":{"target":"notepad"}}]}}"#;
        let result = generate_automation_with_infer(
            "launch Notepad snapped left",
            Some("open notepad"),
            &MockInfer(json),
            BUILTINS,
        )
        .unwrap();
        assert_eq!(result.kind, ComposerKind::Command);
        assert_eq!(
            result.command.unwrap().trigger_phrases,
            vec!["open notepad"]
        );
    }

    #[test]
    fn generate_with_mock_infer_open_notepad() {
        let json = r#"{"kind":"command","confidence":0.9,"summary":"Open Notepad","command":{"trigger_phrases":["open notepad"],"match_mode":"phrase","actions":[{"open_target":{"target":"notepad"}}]}}"#;
        let result =
            generate_automation_with_infer("open notepad", None, &MockInfer(json), BUILTINS).unwrap();
        assert_eq!(result.kind, ComposerKind::Command);
        assert_eq!(
            result.command.unwrap().trigger_phrases,
            vec!["open notepad"]
        );
    }

    #[test]
    fn parse_command_with_empty_object_unit_actions() {
        let json = r#"{"kind":"command","confidence":0.85,"summary":"Read clipboard","command":{"trigger_phrases":["copy status"],"match_mode":"phrase","actions":[{"get_clipboard":{}}]}}"#;
        let raw = parse_composer_output(json).expect("get_clipboard {{}} should parse");
        let result = finalize_automation_result(raw, "copy status", BUILTINS).unwrap();
        assert!(matches!(
            &result.command.unwrap().actions[0],
            Action::GetClipboard {}
        ));
    }

    #[test]
    fn repair_pass_fixes_invalid_json() {
        let bad = r#"{"kind":"command","confidence":0.8,"summary":"clip","command":{"trigger_phrases":["clip"],"match_mode":"phrase","actions":[{"get_clipboard":{}}]}}"#;
        // Simulate legacy null form that still fails — repair should fix to {}
        let bad_null = r#"{"kind":"command","confidence":0.8,"summary":"clip","command":{"trigger_phrases":["clip"],"match_mode":"phrase","actions":[{"get_clipboard":null}]}}"#;
        let good = bad;
        let calls = std::cell::Cell::new(0u32);
        struct RepairMock<'a> {
            bad: &'a str,
            good: &'a str,
            calls: &'a std::cell::Cell<u32>,
        }
        impl ComposerInfer for RepairMock<'_> {
            fn infer(&self, _prompt: &str) -> Result<String, String> {
                let n = self.calls.get();
                self.calls.set(n + 1);
                if n == 0 {
                    Ok(self.bad.to_string())
                } else {
                    Ok(self.good.to_string())
                }
            }
        }
        let mock = RepairMock {
            bad: bad_null,
            good,
            calls: &calls,
        };
        let result = generate_automation_with_infer("clip", None, &mock, BUILTINS).unwrap();
        assert_eq!(calls.get(), 2);
        assert!(matches!(
            &result.command.unwrap().actions[0],
            Action::GetClipboard {}
        ));
    }

    #[test]
    fn repair_pass_fixes_validation_error() {
        let bad = r#"{"kind":"command","confidence":0.8,"summary":"bad","command":{"trigger":"","trigger_phrases":[],"match_mode":"phrase","actions":[{"speak":{"text":"hi"}}]}}"#;
        let good = r#"{"kind":"command","confidence":0.8,"summary":"fixed","command":{"trigger":"hi","match_mode":"phrase","actions":[{"speak":{"text":"hi"}}]}}"#;
        let calls = std::cell::Cell::new(0u32);
        struct RepairMock<'a> {
            bad: &'a str,
            good: &'a str,
            calls: &'a std::cell::Cell<u32>,
        }
        impl ComposerInfer for RepairMock<'_> {
            fn infer(&self, _prompt: &str) -> Result<String, String> {
                let n = self.calls.get();
                self.calls.set(n + 1);
                if n == 0 {
                    Ok(self.bad.to_string())
                } else {
                    Ok(self.good.to_string())
                }
            }
        }
        let mock = RepairMock {
            bad,
            good,
            calls: &calls,
        };
        let result = generate_automation_with_infer("say hi", None, &mock, BUILTINS).unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(result.summary, "fixed");
    }

    #[test]
    fn low_confidence_adds_warning_without_rejecting() {
        let json = r#"{
            "kind": "command",
            "confidence": 0.3,
            "summary": "Maybe",
            "command": {
                "trigger_phrases": ["test"],
                "match_mode": "phrase",
                "actions": [{"speak": {"text": "hi"}}]
            }
        }"#;
        let raw = parse_composer_output(json).unwrap();
        let result = finalize_automation_result(raw, "test", BUILTINS).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("Low confidence")));
    }

    #[test]
    fn extract_trigger_splits_on_then() {
        let trigger = extract_trigger_from_description(
            "when I say open notepad then open notepad",
        )
        .unwrap();
        assert_eq!(trigger, "open notepad");
    }

    #[test]
    fn dedupe_removes_consecutive_duplicate_open_target() {
        let duplicate = Action::OpenTarget {
            target: "notepad".into(),
            placement: None,
        };
        let mut actions = vec![duplicate.clone(), duplicate];
        let warnings = dedupe_consecutive_actions(&mut actions);
        assert_eq!(actions.len(), 1);
        assert_eq!(warnings, vec!["Removed duplicate action.".to_string()]);
    }

    #[test]
    fn align_trigger_fixes_then_in_trigger_phrase() {
        let mut cmd = NewCommandNode {
            name: "open notepad then open notepad".into(),
            trigger_phrases: vec!["open notepad then open notepad".into()],
            actions: vec![Action::OpenTarget {
                target: "notepad".into(),
                placement: None,
            }],
            enabled: true,
            fuzzy_threshold_pct: 0,
            match_mode: MatchMode::Phrase,
        };
        let warnings = align_trigger_from_description(
            &mut cmd,
            "when I say open notepad then open notepad",
        );
        assert_eq!(cmd.trigger_phrases, vec!["open notepad"]);
        assert_eq!(cmd.name, "open notepad");
        assert!(warnings.iter().any(|w| w.contains("Adjusted trigger")));
    }

    #[test]
    fn finalize_dedupes_duplicate_open_notepad_mock() {
        let json = r#"{
            "kind": "command",
            "confidence": 0.9,
            "summary": "Open Notepad",
            "command": {
                "trigger_phrases": ["open notepad then open notepad"],
                "match_mode": "phrase",
                "actions": [
                    {"open_target": {"target": "notepad"}},
                    {"open_target": {"target": "notepad"}}
                ]
            }
        }"#;
        let raw = parse_composer_output(json).unwrap();
        let result = finalize_automation_result(
            raw,
            "when I say open notepad then open notepad",
            BUILTINS,
        )
        .unwrap();
        let cmd = result.command.unwrap();
        assert_eq!(cmd.actions.len(), 1);
        assert_eq!(cmd.trigger_phrases, vec!["open notepad"]);
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("Removed duplicate action")));
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("Adjusted trigger")));
    }
}

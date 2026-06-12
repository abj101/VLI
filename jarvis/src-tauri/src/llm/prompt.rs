//! Build router prompts from enabled tool definitions.

use crate::db::{ToolDefinition, ToolParameter};
use serde_json::{json, Value};

/// JSON-schema-style parameter object for the tool catalog shown to the model.
pub fn parameter_schema(param: &ToolParameter) -> Value {
    let mut schema = serde_json::Map::new();
    match param.param_type.as_str() {
        "enum" if !param.enum_values.is_empty() => {
            schema.insert("type".into(), json!("string"));
            schema.insert("enum".into(), json!(param.enum_values));
        }
        "boolean" => {
            schema.insert("type".into(), json!("boolean"));
        }
        "number" | "integer" => {
            schema.insert("type".into(), json!(param.param_type));
        }
        _ => {
            schema.insert("type".into(), json!("string"));
        }
    }
    if let Some(desc) = param.description.as_ref().filter(|d| !d.trim().is_empty()) {
        schema.insert("description".into(), json!(desc));
    }
    Value::Object(schema)
}

/// Catalog entry for one enabled tool (name, description, JSON Schema params).
pub fn tool_catalog_entry(tool: &ToolDefinition) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for param in &tool.parameters {
        properties.insert(param.name.clone(), parameter_schema(param));
        if param.required {
            required.push(Value::String(param.name.clone()));
        }
    }
    let mut params_schema = serde_json::Map::new();
    params_schema.insert("type".into(), json!("object"));
    params_schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        params_schema.insert("required".into(), Value::Array(required));
    }
    params_schema.insert("additionalProperties".into(), json!(false));

    json!({
        "name": tool.name,
        "description": tool.description,
        "parameters": Value::Object(params_schema),
    })
}

/// Enabled tools only, sorted by name for stable prompts.
pub fn build_tool_catalog(tools: &[ToolDefinition]) -> Vec<Value> {
    let mut enabled: Vec<&ToolDefinition> = tools.iter().filter(|t| t.enabled).collect();
    enabled.sort_by(|a, b| a.name.cmp(&b.name));
    enabled.iter().map(|t| tool_catalog_entry(t)).collect()
}

const SYSTEM_INSTRUCTIONS: &str = r#"You are a voice-command router. Given the user transcript and available tools, pick one or more tool calls (max 3).

Respond with a single JSON object only — no markdown, no explanation:
{ "tool_calls": [ { "tool": "<tool_name>", "args": { ... } }, ... ], "confidence": <0.0-1.0> }

Legacy single-call shape is also accepted:
{ "tool": "<tool_name>", "args": { ... }, "confidence": <0.0-1.0> }

Rules:
- Prefer "tool_calls" for multi-step intents (e.g. open app then snap window).
- At most 3 tool calls per response.
- Each "tool" must be one of the catalog tool names.
- Each "args" object must match that tool's parameter schema.
- "confidence" is how sure you are about the whole plan (0.0–1.0).
- Omit optional args when not needed.
- If nothing matches, use the closest tool with low confidence."#;

/// Full prompt text for local inference (chat template applied separately when available).
pub fn build_router_prompt(transcript: &str, tools: &[ToolDefinition]) -> String {
    let catalog = build_tool_catalog(tools);
    let catalog_json =
        serde_json::to_string_pretty(&catalog).unwrap_or_else(|_| "[]".to_string());
    format!(
        "{SYSTEM_INSTRUCTIONS}\n\nTools:\n{catalog_json}\n\nTranscript: {transcript}\n\nJSON:"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Action, ToolParameter};

    fn sample_open_target() -> ToolDefinition {
        ToolDefinition {
            id: 1,
            name: "open_target".into(),
            display_name: "Open".into(),
            description: "Opens an app or site".into(),
            parameters: vec![
                ToolParameter {
                    name: "target".into(),
                    param_type: "string".into(),
                    description: Some("App or site name".into()),
                    required: true,
                    enum_values: vec![],
                },
                ToolParameter {
                    name: "placement".into(),
                    param_type: "enum".into(),
                    description: None,
                    required: false,
                    enum_values: vec!["left_half".into(), "right_half".into()],
                },
            ],
            actions: vec![Action::OpenTarget {
                target: "{{target}}".into(),
                placement: Some("{{placement}}".into()),
            }],
            enabled: true,
            builtin: true,
            created_at: String::new(),
        }
    }

    #[test]
    fn catalog_skips_disabled_tools() {
        let mut disabled = sample_open_target();
        disabled.enabled = false;
        let catalog = build_tool_catalog(&[disabled]);
        assert!(catalog.is_empty());
    }

    #[test]
    fn prompt_includes_transcript_and_tool_name() {
        let prompt = build_router_prompt("put brave on the left", &[sample_open_target()]);
        assert!(prompt.contains("put brave on the left"));
        assert!(prompt.contains("\"name\": \"open_target\""));
        assert!(prompt.contains("left_half"));
    }
}

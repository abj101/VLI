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

const SYSTEM_INSTRUCTIONS: &str = r#"You route voice commands to available tools. Choose one or more tool calls (maximum 3).

Return exactly one JSON object with `tool_calls` and `confidence`. Every call must name a catalog tool and provide arguments that match its parameter schema. Omit optional arguments you do not need. Set confidence from 0.0 to 1.0 based on the whole plan. If nothing matches, return the closest tool with low confidence.

Routing:
- Use `open_target` for installed apps and generic "open X" requests.
- Use `open_url` only when the transcript contains an explicit URL or domain. Never guess a URL for an app name.
- Use `shortcut` for editing, tab, and window keyboard actions such as copy or close tab.
- Use `media` for playback and volume controls.
- Use `web_search` when the user asks to search, Google, or look something up.
- Use `type_text` when the user dictates text to enter in the current app.
- Use `say` only when the user asks Jarvis to speak something.

Examples:
Transcript: "copy that"
{"tool_calls":[{"tool":"shortcut","args":{"name":"copy"}}],"confidence":0.9}

Transcript: "open spotify and play"
{"tool_calls":[{"tool":"open_target","args":{"target":"spotify"}},{"tool":"media","args":{"control":"play_pause"}}],"confidence":0.9}

Transcript: "search for rust traits"
{"tool_calls":[{"tool":"web_search","args":{"query":"rust traits"}}],"confidence":0.9}"#;

/// Build the llama-server constrained decoding schema for enabled tools.
pub fn build_router_schema(tools: &[ToolDefinition]) -> Value {
    let variants = tools
        .iter()
        .filter(|tool| tool.enabled)
        .map(|tool| {
            json!({
                "type": "object",
                "properties": {
                    "tool": { "const": tool.name },
                    "args": tool_catalog_entry(tool)["parameters"].clone()
                },
                "required": ["tool", "args"],
                "additionalProperties": false
            })
        })
        .collect::<Vec<_>>();

    json!({
        "type": "object",
        "properties": {
            "tool_calls": {
                "type": "array",
                "minItems": 1,
                "maxItems": crate::llm::router::MAX_ROUTER_TOOL_CALLS,
                "items": { "anyOf": variants }
            },
            "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
        },
        "required": ["tool_calls", "confidence"],
        "additionalProperties": false
    })
}
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
        assert!(prompt.contains("Use `shortcut`"));
        assert!(prompt.contains("Transcript: put brave on the left\n\nJSON:"));
    }

    #[test]
    fn router_schema_limits_and_enumerates_enabled_tools() {
        let first = sample_open_target();
        let mut second = sample_open_target();
        second.name = "open_url".into();
        let schema = build_router_schema(&[first, second]);
        let variants = schema["properties"]["tool_calls"]["items"]["anyOf"]
            .as_array()
            .unwrap();

        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0]["properties"]["tool"]["const"], "open_target");
        assert_eq!(variants[1]["properties"]["tool"]["const"], "open_url");
        assert_eq!(
            schema["properties"]["tool_calls"]["maxItems"],
            crate::llm::router::MAX_ROUTER_TOOL_CALLS
        );
    }
}

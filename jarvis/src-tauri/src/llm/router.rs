//! Parse and validate LLM router JSON output.

use crate::db::{get_tool_by_name, ToolDefinition, ToolParameter};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Maximum tool calls in one router response (multi-step intents).
pub const MAX_ROUTER_TOOL_CALLS: usize = 3;

/// One validated tool invocation (no per-call confidence).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterToolCall {
    pub tool: String,
    pub args: HashMap<String, String>,
}

/// Successful router output after parse + schema validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterRouteResult {
    pub tool_calls: Vec<RouterToolCall>,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterErrorCode {
    FeatureDisabled,
    Tier2Disabled,
    ModelMissing,
    ModelLoading,
    InvalidJson,
    LowConfidence,
    UnknownTool,
    ToolDisabled,
    SchemaInvalid,
    InferFailed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RouterError {
    pub code: RouterErrorCode,
    pub message: String,
}

impl RouterError {
    pub fn into_string(self) -> String {
        serde_json::to_string(&self).unwrap_or(self.message)
    }
}

/// Abstraction for unit tests (mock) and local GGUF inference.
pub trait RouterInfer {
    fn infer(&self, prompt: &str) -> Result<String, String>;
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawRouterOutput {
    #[serde(default)]
    tool: String,
    #[serde(default)]
    args: Map<String, Value>,
    #[serde(default)]
    tool_calls: Vec<RawRouterToolCall>,
    confidence: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct RawRouterToolCall {
    tool: String,
    args: Map<String, Value>,
}

/// Pull the first balanced `{ ... }` object from model text.
pub fn extract_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    return Some(text[start..end].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

pub fn parse_router_output(raw: &str) -> Result<RawRouterOutput, RouterError> {
    let trimmed = raw.trim();
    let json_text = extract_json_object(trimmed).unwrap_or_else(|| trimmed.to_string());
    serde_json::from_str::<RawRouterOutput>(&json_text).map_err(|e| RouterError {
        code: RouterErrorCode::InvalidJson,
        message: format!("router output is not valid JSON: {e}"),
    })
}

fn validate_param_value(param: &ToolParameter, value: &Value) -> Result<String, RouterError> {
    match param.param_type.as_str() {
        "boolean" => match value {
            Value::Bool(b) => Ok(b.to_string()),
            Value::String(s) if s == "true" || s == "false" => Ok(s.clone()),
            _ => Err(RouterError {
                code: RouterErrorCode::SchemaInvalid,
                message: format!("argument `{}` must be a boolean", param.name),
            }),
        },
        "enum" if !param.enum_values.is_empty() => {
            let s = value_as_string(value)?;
            if param.enum_values.iter().any(|v| v == &s) {
                Ok(s)
            } else {
                Err(RouterError {
                    code: RouterErrorCode::SchemaInvalid,
                    message: format!(
                        "argument `{}` must be one of [{}]",
                        param.name,
                        param.enum_values.join(", ")
                    ),
                })
            }
        }
        "number" | "integer" => match value {
            Value::Number(n) => Ok(n.to_string()),
            Value::String(s) if !s.trim().is_empty() => Ok(s.trim().to_string()),
            _ => Err(RouterError {
                code: RouterErrorCode::SchemaInvalid,
                message: format!("argument `{}` must be a number", param.name),
            }),
        },
        _ => {
            let s = value_as_string(value)?;
            if s.trim().is_empty() {
                Err(RouterError {
                    code: RouterErrorCode::SchemaInvalid,
                    message: format!("argument `{}` must be a non-empty string", param.name),
                })
            } else {
                Ok(s.trim().to_string())
            }
        }
    }
}

fn value_as_string(value: &Value) -> Result<String, RouterError> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => Err(RouterError {
            code: RouterErrorCode::SchemaInvalid,
            message: "expected a scalar argument value".into(),
        }),
    }
}

pub fn validate_router_call(
    conn: &Connection,
    raw: &RawRouterOutput,
    confidence_threshold: f32,
) -> Result<RouterRouteResult, RouterError> {
    let confidence = validate_confidence(raw.confidence, confidence_threshold)?;
    let tool_calls = normalize_raw_tool_calls(raw)?;
    if tool_calls.is_empty() {
        return Err(RouterError {
            code: RouterErrorCode::SchemaInvalid,
            message: "at least one tool call is required".into(),
        });
    }
    if tool_calls.len() > MAX_ROUTER_TOOL_CALLS {
        return Err(RouterError {
            code: RouterErrorCode::SchemaInvalid,
            message: format!(
                "too many tool calls (max {MAX_ROUTER_TOOL_CALLS}); multi-step sequence not supported"
            ),
        });
    }
    let mut validated = Vec::with_capacity(tool_calls.len());
    for raw_call in tool_calls {
        validated.push(validate_single_tool_call(conn, &raw_call)?);
    }
    Ok(RouterRouteResult {
        tool_calls: validated,
        confidence,
    })
}

fn validate_confidence(confidence: f64, threshold: f32) -> Result<f32, RouterError> {
    if !(confidence.is_finite() && (0.0..=1.0).contains(&confidence)) {
        return Err(RouterError {
            code: RouterErrorCode::SchemaInvalid,
            message: "confidence must be a number between 0 and 1".into(),
        });
    }
    let confidence = confidence as f32;
    if confidence < threshold {
        return Err(RouterError {
            code: RouterErrorCode::LowConfidence,
            message: format!("confidence {confidence:.2} is below threshold {threshold:.2}"),
        });
    }
    Ok(confidence)
}

fn normalize_raw_tool_calls(raw: &RawRouterOutput) -> Result<Vec<RawRouterToolCall>, RouterError> {
    if !raw.tool_calls.is_empty() {
        return Ok(raw.tool_calls.clone());
    }
    let tool_name = raw.tool.trim();
    if tool_name.is_empty() {
        return Ok(vec![]);
    }
    Ok(vec![RawRouterToolCall {
        tool: tool_name.to_string(),
        args: raw.args.clone(),
    }])
}

fn validate_single_tool_call(
    conn: &Connection,
    raw: &RawRouterToolCall,
) -> Result<RouterToolCall, RouterError> {
    let tool_name = raw.tool.trim();
    if tool_name.is_empty() {
        return Err(RouterError {
            code: RouterErrorCode::SchemaInvalid,
            message: "tool name is required".into(),
        });
    }
    let tool = get_tool_by_name(conn, tool_name)
        .map_err(|e| RouterError {
            code: RouterErrorCode::SchemaInvalid,
            message: e.to_string(),
        })?
        .ok_or_else(|| RouterError {
            code: RouterErrorCode::UnknownTool,
            message: format!("unknown tool `{tool_name}`"),
        })?;
    if !tool.enabled {
        return Err(RouterError {
            code: RouterErrorCode::ToolDisabled,
            message: format!("tool `{tool_name}` is disabled"),
        });
    }
    let args = validate_args_against_tool(&tool, &raw.args)?;
    Ok(RouterToolCall {
        tool: tool.name,
        args,
    })
}

pub fn validate_args_against_tool(
    tool: &ToolDefinition,
    raw_args: &Map<String, Value>,
) -> Result<HashMap<String, String>, RouterError> {
    let known: std::collections::HashSet<&str> =
        tool.parameters.iter().map(|p| p.name.as_str()).collect();
    for key in raw_args.keys() {
        if !known.contains(key.as_str()) {
            return Err(RouterError {
                code: RouterErrorCode::SchemaInvalid,
                message: format!("unknown argument `{key}` for tool `{}`", tool.name),
            });
        }
    }

    let mut out = HashMap::new();
    for param in &tool.parameters {
        match raw_args.get(&param.name) {
            Some(value) => {
                let parsed = validate_param_value(param, value)?;
                if !parsed.is_empty() {
                    out.insert(param.name.clone(), parsed);
                }
            }
            None if param.required => {
                return Err(RouterError {
                    code: RouterErrorCode::SchemaInvalid,
                    message: format!(
                        "missing required argument `{}` for tool `{}`",
                        param.name, tool.name
                    ),
                });
            }
            None => {}
        }
    }
    Ok(out)
}

pub fn route_transcript_with_infer(
    conn: &Connection,
    tools: &[ToolDefinition],
    transcript: &str,
    confidence_threshold: f32,
    infer: &impl RouterInfer,
) -> Result<RouterRouteResult, RouterError> {
    let prompt = crate::llm::prompt::build_router_prompt(transcript, tools);
    let completion = infer.infer(&prompt).map_err(|message| RouterError {
        code: RouterErrorCode::InferFailed,
        message,
    })?;
    let raw = parse_router_output(&completion)?;
    validate_router_call(conn, &raw, confidence_threshold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::tempdir;

    struct MockInfer(&'static str);

    impl RouterInfer for MockInfer {
        fn infer(&self, _prompt: &str) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }

    fn test_conn() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("router-test.db");
        init_db(&path).unwrap();
        (dir, Connection::open(&path).unwrap())
    }

    fn open_target_tool(conn: &Connection) -> ToolDefinition {
        crate::db::get_tool_by_name(conn, "open_target")
            .unwrap()
            .expect("open_target builtin")
    }

    #[test]
    fn extract_json_object_handles_wrapping_text() {
        let raw = "Sure! {\"tool\":\"open_target\",\"args\":{\"target\":\"brave\"},\"confidence\":0.9}";
        let json = extract_json_object(raw).expect("json");
        assert!(json.contains("\"tool\":\"open_target\""));
    }

    #[test]
    fn route_brave_left_placement() {
        let (_dir, conn) = test_conn();
        let tool = open_target_tool(&conn);
        let mock = MockInfer(
            r#"{"tool":"open_target","args":{"target":"brave","placement":"left_half"},"confidence":0.92}"#,
        );
        let result = route_transcript_with_infer(
            &conn,
            &[tool],
            "put brave on the left",
            0.7,
            &mock,
        )
        .expect("route");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].tool, "open_target");
        assert_eq!(
            result.tool_calls[0].args.get("target").map(String::as_str),
            Some("brave")
        );
        assert_eq!(
            result.tool_calls[0].args.get("placement").map(String::as_str),
            Some("left_half")
        );
        assert!(result.confidence >= 0.7);
    }

    #[test]
    fn low_confidence_rejected() {
        let (_dir, conn) = test_conn();
        let tool = open_target_tool(&conn);
        let mock = MockInfer(
            r#"{"tool":"open_target","args":{"target":"brave"},"confidence":0.4}"#,
        );
        let err = route_transcript_with_infer(&conn, &[tool], "open brave", 0.7, &mock).unwrap_err();
        assert_eq!(err.code, RouterErrorCode::LowConfidence);
    }

    #[test]
    fn invalid_json_rejected() {
        let (_dir, conn) = test_conn();
        let tool = open_target_tool(&conn);
        let mock = MockInfer("not json at all");
        let err =
            route_transcript_with_infer(&conn, &[tool], "open brave", 0.7, &mock).unwrap_err();
        assert_eq!(err.code, RouterErrorCode::InvalidJson);
    }

    #[test]
    fn unknown_arg_rejected() {
        let (_dir, conn) = test_conn();
        let raw = RawRouterOutput {
            tool: "open_target".into(),
            args: Map::from_iter([("bogus".into(), json!("x"))]),
            tool_calls: vec![],
            confidence: 0.9,
        };
        let err = validate_router_call(&conn, &raw, 0.7).unwrap_err();
        assert_eq!(err.code, RouterErrorCode::SchemaInvalid);
    }

    #[test]
    fn enum_placement_validated() {
        let (_dir, conn) = test_conn();
        let raw = RawRouterOutput {
            tool: "open_target".into(),
            args: Map::from_iter([
                ("target".into(), json!("brave")),
                ("placement".into(), json!("top_left")),
            ]),
            tool_calls: vec![],
            confidence: 0.9,
        };
        let err = validate_router_call(&conn, &raw, 0.7).unwrap_err();
        assert_eq!(err.code, RouterErrorCode::SchemaInvalid);
    }

    #[test]
    fn multi_step_tool_calls_up_to_three() {
        let (_dir, conn) = test_conn();
        let raw = RawRouterOutput {
            tool: String::new(),
            args: Map::new(),
            tool_calls: vec![
                RawRouterToolCall {
                    tool: "open_target".into(),
                    args: Map::from_iter([("target".into(), json!("brave"))]),
                },
                RawRouterToolCall {
                    tool: "snap_window".into(),
                    args: Map::from_iter([("zone".into(), json!("maximize"))]),
                },
            ],
            confidence: 0.88,
        };
        let result = validate_router_call(&conn, &raw, 0.7).expect("multi-step");
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].tool, "open_target");
        assert_eq!(result.tool_calls[1].tool, "snap_window");
    }

    #[test]
    fn rejects_more_than_max_tool_calls() {
        let (_dir, conn) = test_conn();
        let raw = RawRouterOutput {
            tool: String::new(),
            args: Map::new(),
            tool_calls: (0..4)
                .map(|_| RawRouterToolCall {
                    tool: "open_url".into(),
                    args: Map::from_iter([("url".into(), json!("https://example.com"))]),
                })
                .collect(),
            confidence: 0.9,
        };
        let err = validate_router_call(&conn, &raw, 0.7).unwrap_err();
        assert_eq!(err.code, RouterErrorCode::SchemaInvalid);
        assert!(err.message.contains("max 3"));
    }
}

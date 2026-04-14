//! AI mode HTTP client — configurable Anthropic, OpenAI-compatible, or Ollama endpoints.
#![allow(dead_code)] // Exported for T4-6 orchestrator; not called from `lib` yet.

use crate::db::{Action, CommandNode};
use reqwest::Client;
use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;

const ANTHROPIC_MODEL_DEFAULT: &str = "claude-haiku-4-5";
const MAX_TOKENS: u32 = 512;

/// Default system instructions when `CommandNode.sub_prompt` is missing or empty.
const DEFAULT_SYSTEM_PROMPT: &str = r#"You are JARVIS. Reply with a single JSON object only (no markdown fences, no other text). Shape:
{"text":"string shown to the user","actions":[ ... ]}
`actions` is an array of action objects. Each action uses one variant name in snake_case with its fields, e.g. {"open_app":{"name":"Notepad","path":"notepad"}}, {"open_url":{"url":"https://example.com"}}, {"run_script":{"script":"cmd","args":[]}}, {"send_keys":{"keys":"ctrl+n"}}, {"wait":{"ms":500}}, {"speak":{"text":"..."}}, {"sub_prompt":{"prompt":"..."}}.
If no actions are needed, use "actions":[]."#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiResponse {
    pub text: String,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderKind {
    /// Anthropic Messages API (`/v1/messages`).
    #[default]
    AnthropicMessages,
    /// OpenAI Chat Completions (`/v1/chat/completions`) — LM Studio, vLLM, OpenAI-compatible proxies.
    OpenAiChatCompletions,
    /// Ollama native `/api/chat` (local; no API key by default).
    OllamaChat,
}

/// Where and how to call the model. Persist in settings (T4-4) for BYOK + local endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiEndpointConfig {
    pub kind: AiProviderKind,
    /// Base URL without trailing slash, e.g. `https://api.anthropic.com`, `http://127.0.0.1:1234`, `http://localhost:11434`.
    /// If the URL already ends with a known path (`/v1/messages`, `/v1/chat/completions`, `/api/chat`), it is used as the full POST URL.
    pub base_url: String,
    /// Model id in the provider’s request body (`model` field).
    pub model: String,
}

impl Default for AiEndpointConfig {
    fn default() -> Self {
        Self {
            kind: AiProviderKind::AnthropicMessages,
            base_url: "https://api.anthropic.com".to_string(),
            model: ANTHROPIC_MODEL_DEFAULT.to_string(),
        }
    }
}

impl AiEndpointConfig {
    /// Resolves the POST URL for this provider and base.
    pub fn resolve_post_url(&self) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        let lower = base.to_lowercase();
        if lower.ends_with("/v1/messages")
            || lower.ends_with("/v1/chat/completions")
            || lower.ends_with("/api/chat")
        {
            return base.to_string();
        }
        match self.kind {
            AiProviderKind::AnthropicMessages => {
                if lower.ends_with("/v1") {
                    format!("{}/messages", base)
                } else {
                    format!("{}/v1/messages", base)
                }
            }
            AiProviderKind::OpenAiChatCompletions => {
                if lower.ends_with("/v1") {
                    format!("{}/chat/completions", base)
                } else {
                    format!("{}/v1/chat/completions", base)
                }
            }
            AiProviderKind::OllamaChat => format!("{}/api/chat", base),
        }
    }
}

#[derive(Debug, Error)]
pub enum AiError {
    #[error("AI request timed out")]
    Timeout,
    #[error("request failed: {0}")]
    Request(String),
    #[error("response error: {0}")]
    Response(String),
}

/// Anthropic cloud default (Haiku).
pub async fn run_ai_mode(
    node: &CommandNode,
    transcript: &str,
    api_key: &str,
) -> Result<AiResponse, AiError> {
    run_ai_mode_with_config(node, transcript, api_key, &AiEndpointConfig::default()).await
}

/// Same as [`run_ai_mode`], but uses your endpoint (local OpenAI-compatible, Ollama, or custom Anthropic base).
pub async fn run_ai_mode_with_config(
    node: &CommandNode,
    transcript: &str,
    api_key: &str,
    config: &AiEndpointConfig,
) -> Result<AiResponse, AiError> {
    run_ai_mode_inner(config, node, transcript, api_key, Duration::from_secs(10)).await
}

async fn run_ai_mode_inner(
    config: &AiEndpointConfig,
    node: &CommandNode,
    transcript: &str,
    api_key: &str,
    timeout: Duration,
) -> Result<AiResponse, AiError> {
    let key = api_key.trim();
    if config.kind == AiProviderKind::AnthropicMessages && key.is_empty() {
        return Err(AiError::Request(
            "API key is required for Anthropic Messages".into(),
        ));
    }

    let system = node
        .sub_prompt
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());

    let user_content = format!("User transcript:\n{transcript}");
    let post_url = config.resolve_post_url();

    let client = Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| AiError::Request(e.to_string()))?;

    let request = match config.kind {
        AiProviderKind::AnthropicMessages => {
            let payload = serde_json::json!({
                "model": config.model,
                "max_tokens": MAX_TOKENS,
                "system": system,
                "messages": [
                    {
                        "role": "user",
                        "content": user_content
                    }
                ]
            });
            client
                .post(&post_url)
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&payload)
        }
        AiProviderKind::OpenAiChatCompletions => {
            let payload = serde_json::json!({
                "model": config.model,
                "max_tokens": MAX_TOKENS,
                "messages": [
                    { "role": "system", "content": system },
                    { "role": "user", "content": user_content }
                ]
            });
            apply_optional_bearer(
                client
                    .post(&post_url)
                    .header("content-type", "application/json")
                    .json(&payload),
                key,
            )
        }
        AiProviderKind::OllamaChat => {
            let payload = serde_json::json!({
                "model": config.model,
                "messages": [
                    { "role": "system", "content": system },
                    { "role": "user", "content": user_content }
                ],
                "stream": false
            });
            apply_optional_bearer(
                client
                    .post(&post_url)
                    .header("content-type", "application/json")
                    .json(&payload),
                key,
            )
        }
    };

    let response = request.send().await.map_err(|e| {
        if e.is_timeout() {
            AiError::Timeout
        } else {
            AiError::Request(e.to_string())
        }
    })?;

    let status = response.status();
    let body_text = response
        .text()
        .await
        .map_err(|e| AiError::Response(e.to_string()))?;

    if !status.is_success() {
        return Err(AiError::Response(format!(
            "HTTP {} (body omitted from error)",
            status.as_u16()
        )));
    }

    let value: Value =
        serde_json::from_str(&body_text).map_err(|e| AiError::Response(e.to_string()))?;

    let assistant_text = extract_assistant_text(config.kind, &value)
        .ok_or_else(|| AiError::Response("missing assistant text in model response".into()))?;

    Ok(parse_ai_payload(&assistant_text))
}

/// OpenAI-compatible and Ollama: send `Authorization: Bearer` when `key` is non-empty.
fn apply_optional_bearer(mut req: RequestBuilder, key: &str) -> RequestBuilder {
    if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    req
}

fn extract_assistant_text(kind: AiProviderKind, value: &Value) -> Option<String> {
    match kind {
        AiProviderKind::AnthropicMessages => extract_anthropic_text(value),
        AiProviderKind::OpenAiChatCompletions => extract_openai_text(value),
        AiProviderKind::OllamaChat => extract_ollama_text(value),
    }
}

fn extract_anthropic_text(value: &Value) -> Option<String> {
    value
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn extract_openai_text(value: &Value) -> Option<String> {
    let content = value
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?;
    match content {
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        // Some APIs return structured content blocks
        Value::Array(parts) => {
            let mut out = String::new();
            for p in parts {
                if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
                    out.push_str(t);
                }
            }
            let t = out.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        _ => None,
    }
}

fn extract_ollama_text(value: &Value) -> Option<String> {
    value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Parse model output: try JSON → `AiResponse`; on failure return raw text and no actions.
fn parse_ai_payload(assistant_text: &str) -> AiResponse {
    let trimmed = assistant_text.trim();
    if let Ok(parsed) = serde_json::from_str::<AiResponse>(trimmed) {
        return parsed;
    }
    if let Some(stripped) = strip_markdown_json_fence(trimmed) {
        if let Ok(parsed) = serde_json::from_str::<AiResponse>(stripped) {
            return parsed;
        }
    }
    AiResponse {
        text: trimmed.to_string(),
        actions: vec![],
    }
}

fn strip_markdown_json_fence(s: &str) -> Option<&str> {
    let s = s.strip_prefix("```json")?;
    let s = s.trim_start_matches(['\n', '\r']);
    s.strip_suffix("```").map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::CommandNode;
    use httpmock::prelude::*;

    fn sample_node(sub_prompt: Option<String>) -> CommandNode {
        CommandNode {
            id: 1,
            name: "test".into(),
            trigger_phrases: vec![],
            actions: vec![],
            enabled: true,
            fuzzy_threshold_pct: 80,
            ai_mode: true,
            sub_prompt,
            created_at: "".into(),
        }
    }

    fn anthropic_ok_body(inner_json: &str) -> String {
        let escaped = serde_json::to_string(inner_json).unwrap();
        format!(r#"{{"content":[{{"type":"text","text":{}}}]}}"#, escaped)
    }

    #[tokio::test]
    async fn anthropic_success_parses_structured_json() {
        let server = MockServer::start();
        let inner = r#"{"text":"Done","actions":[{"open_url":{"url":"https://example.com"}}]}"#;
        let _m = server.mock(|when, then| {
            when.method(POST).path("/v1/messages");
            then.status(200)
                .header("content-type", "application/json")
                .body(anthropic_ok_body(inner));
        });

        let cfg = AiEndpointConfig {
            kind: AiProviderKind::AnthropicMessages,
            base_url: server.base_url(),
            model: "claude-haiku-4-5".into(),
        };
        let node = sample_node(None);
        let out = run_ai_mode_inner(
            &cfg,
            &node,
            "open example",
            "test-key-not-real",
            Duration::from_secs(5),
        )
        .await
        .expect("ok");

        assert_eq!(out.text, "Done");
        assert_eq!(
            out.actions,
            vec![Action::OpenUrl {
                url: "https://example.com".into()
            }]
        );
    }

    #[tokio::test]
    async fn openai_compatible_parses_choices_content() {
        let server = MockServer::start();
        let inner = r#"{"text":"ok","actions":[]}"#;
        let body = format!(
            r#"{{"choices":[{{"message":{{"role":"assistant","content":{}}}}}]}}"#,
            serde_json::to_string(inner).unwrap()
        );
        let _m = server.mock(|when, then| {
            when.method(POST).path("/v1/chat/completions");
            then.status(200)
                .header("content-type", "application/json")
                .body(body);
        });

        let cfg = AiEndpointConfig {
            kind: AiProviderKind::OpenAiChatCompletions,
            base_url: server.base_url(),
            model: "local-model".into(),
        };
        let node = sample_node(None);
        let out = run_ai_mode_inner(&cfg, &node, "hi", "", Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(out.text, "ok");
        assert!(out.actions.is_empty());
    }

    #[tokio::test]
    async fn ollama_parses_message_content() {
        let server = MockServer::start();
        let inner = r#"{"text":"yo","actions":[]}"#;
        let body = format!(
            r#"{{"message":{{"role":"assistant","content":{}}}}}"#,
            serde_json::to_string(inner).unwrap()
        );
        let _m = server.mock(|when, then| {
            when.method(POST).path("/api/chat");
            then.status(200)
                .header("content-type", "application/json")
                .body(body);
        });

        let cfg = AiEndpointConfig {
            kind: AiProviderKind::OllamaChat,
            base_url: server.base_url(),
            model: "llama3".into(),
        };
        let node = sample_node(None);
        let out = run_ai_mode_inner(&cfg, &node, "hi", "", Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(out.text, "yo");
    }

    #[tokio::test]
    async fn resolve_url_full_path_passthrough() {
        let c = AiEndpointConfig {
            kind: AiProviderKind::AnthropicMessages,
            base_url: "http://proxy.local/v1/messages".into(),
            model: "x".into(),
        };
        assert_eq!(c.resolve_post_url(), "http://proxy.local/v1/messages");
    }

    #[tokio::test]
    async fn uses_sub_prompt_as_system() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/messages")
                .body_contains("CUSTOM_SYS");
            then.status(200)
                .header("content-type", "application/json")
                .body(anthropic_ok_body(r#"{"text":"x","actions":[]}"#));
        });

        let cfg = AiEndpointConfig {
            kind: AiProviderKind::AnthropicMessages,
            base_url: server.base_url(),
            model: "claude-haiku-4-5".into(),
        };
        let node = sample_node(Some("CUSTOM_SYS only reply {\"text\":\"x\"}".into()));
        let out = run_ai_mode_inner(&cfg, &node, "hi", "k", Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(out.text, "x");
    }

    #[tokio::test]
    async fn malformed_json_degrades_to_raw_text() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(POST).path("/v1/messages");
            then.status(200)
                .header("content-type", "application/json")
                .body(anthropic_ok_body("not json at all"));
        });

        let cfg = AiEndpointConfig {
            kind: AiProviderKind::AnthropicMessages,
            base_url: server.base_url(),
            model: "claude-haiku-4-5".into(),
        };
        let node = sample_node(None);
        let out = run_ai_mode_inner(&cfg, &node, "t", "k", Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(out.text, "not json at all");
        assert!(out.actions.is_empty());
    }

    #[tokio::test]
    async fn slow_response_returns_timeout() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(POST).path("/v1/messages");
            then.status(200)
                .delay(Duration::from_millis(500))
                .body("{}");
        });

        let cfg = AiEndpointConfig {
            kind: AiProviderKind::AnthropicMessages,
            base_url: server.base_url(),
            model: "claude-haiku-4-5".into(),
        };
        let node = sample_node(None);
        let err = run_ai_mode_inner(&cfg, &node, "t", "k", Duration::from_millis(1))
            .await
            .unwrap_err();

        assert!(matches!(err, AiError::Timeout));
    }

    #[tokio::test]
    async fn default_config_is_anthropic_cloud() {
        let d = AiEndpointConfig::default();
        assert_eq!(d.kind, AiProviderKind::AnthropicMessages);
        assert!(d.base_url.contains("anthropic.com"));
        assert_eq!(
            d.resolve_post_url(),
            "https://api.anthropic.com/v1/messages"
        );
    }
}

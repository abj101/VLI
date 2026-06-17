//! Post-validate LLM router tool calls against app-index and URL rules.

use crate::apps::intent::{classify_open_target, OpenIntent};
use crate::apps::{resolve_target::is_explicit_url, AppEntry};
use crate::db::TargetAlias;
use crate::llm::router::{RouterError, RouterErrorCode, RouterToolCall};
use std::collections::HashMap;

pub fn normalize_router_tool_calls(
    calls: Vec<RouterToolCall>,
    app_index: &[AppEntry],
    aliases: &[TargetAlias],
) -> Result<Vec<RouterToolCall>, RouterError> {
    calls
        .into_iter()
        .map(|call| normalize_single_tool_call(call, app_index, aliases))
        .collect()
}

fn normalize_single_tool_call(
    call: RouterToolCall,
    app_index: &[AppEntry],
    aliases: &[TargetAlias],
) -> Result<RouterToolCall, RouterError> {
    match call.tool.as_str() {
        "open_url" => normalize_open_url_call(call, app_index, aliases),
        "open_target" => normalize_open_target_call(call, app_index, aliases),
        _ => Ok(call),
    }
}

fn normalize_open_url_call(
    call: RouterToolCall,
    app_index: &[AppEntry],
    aliases: &[TargetAlias],
) -> Result<RouterToolCall, RouterError> {
    let url = call
        .args
        .get("url")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RouterError {
            code: RouterErrorCode::SchemaInvalid,
            message: "missing required argument `url` for `open_url`".into(),
        })?;

    let placement = call.args.get("placement").cloned();
    let rewrite_to_target = !is_explicit_url(url)
        || matches!(
            classify_open_target(url, placement.clone(), app_index, aliases),
            OpenIntent::App { .. }
        );

    if rewrite_to_target {
        let mut args = HashMap::new();
        args.insert("target".into(), url.to_string());
        if let Some(p) = placement {
            args.insert("placement".into(), p);
        }
        return Ok(RouterToolCall {
            tool: "open_target".into(),
            args,
        });
    }

    Ok(call)
}

fn normalize_open_target_call(
    call: RouterToolCall,
    app_index: &[AppEntry],
    aliases: &[TargetAlias],
) -> Result<RouterToolCall, RouterError> {
    let target = call
        .args
        .get("target")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RouterError {
            code: RouterErrorCode::SchemaInvalid,
            message: "missing required argument `target` for `open_target`".into(),
        })?;

    let placement = call.args.get("placement").cloned();
    if matches!(
        classify_open_target(target, placement, app_index, aliases),
        OpenIntent::Unknown { .. }
    ) {
        return Err(RouterError {
            code: RouterErrorCode::SchemaInvalid,
            message: format!("unknown open target `{target}`"),
        });
    }

    Ok(call)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{TargetAlias, TargetAliasKind};

    fn brave_entry() -> AppEntry {
        AppEntry {
            display_name: "Brave".into(),
            exe_path: r"C:\Brave\brave.exe".into(),
            icon_data_url: None,
        }
    }

    #[test]
    fn rewrites_open_url_bare_app_name_to_open_target() {
        let index = vec![brave_entry()];
        let calls = vec![RouterToolCall {
            tool: "open_url".into(),
            args: HashMap::from([("url".into(), "brave".into())]),
        }];
        let normalized = normalize_router_tool_calls(calls, &index, &[]).expect("normalized");
        assert_eq!(normalized[0].tool, "open_target");
        assert_eq!(normalized[0].args.get("target").map(String::as_str), Some("brave"));
    }

    #[test]
    fn keeps_open_url_for_explicit_domain() {
        let calls = vec![RouterToolCall {
            tool: "open_url".into(),
            args: HashMap::from([("url".into(), "https://google.com".into())]),
        }];
        let normalized = normalize_router_tool_calls(calls, &[], &[]).expect("normalized");
        assert_eq!(normalized[0].tool, "open_url");
    }

    #[test]
    fn rejects_open_target_unknown_name() {
        let calls = vec![RouterToolCall {
            tool: "open_target".into(),
            args: HashMap::from([("target".into(), "foobar".into())]),
        }];
        let err = normalize_router_tool_calls(calls, &[], &[]).unwrap_err();
        assert_eq!(err.code, RouterErrorCode::SchemaInvalid);
        assert!(err.message.contains("foobar"));
    }

    #[test]
    fn open_target_passes_known_app() {
        let index = vec![brave_entry()];
        let calls = vec![RouterToolCall {
            tool: "open_target".into(),
            args: HashMap::from([("target".into(), "brave".into())]),
        }];
        let normalized = normalize_router_tool_calls(calls, &index, &[]).expect("normalized");
        assert_eq!(normalized[0].tool, "open_target");
    }

    #[test]
    fn open_url_app_in_index_rewrites_even_if_link_like_substring() {
        let index = vec![brave_entry()];
        let calls = vec![RouterToolCall {
            tool: "open_url".into(),
            args: HashMap::from([("url".into(), "brave".into())]),
        }];
        let normalized = normalize_router_tool_calls(calls, &index, &[]).expect("normalized");
        assert_eq!(normalized[0].tool, "open_target");
    }

    #[test]
    fn respects_url_alias_on_open_target() {
        let aliases = vec![TargetAlias {
            spoken: "github".into(),
            kind: TargetAliasKind::Url,
            value: "https://github.com".into(),
        }];
        let calls = vec![RouterToolCall {
            tool: "open_target".into(),
            args: HashMap::from([("target".into(), "github".into())]),
        }];
        let normalized = normalize_router_tool_calls(calls, &[], &aliases).expect("normalized");
        assert_eq!(normalized[0].tool, "open_target");
    }
}

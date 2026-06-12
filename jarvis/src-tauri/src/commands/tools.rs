//! Tool registry execution: load definitions, substitute `{{param}}` templates, run actions.
#![allow(dead_code)] // Phase A foundation; router wires `execute_tool` in Phase C.

use crate::{
    apps::AppEntry,
    commands::executor::{execute_resolved_actions, resolve_action_templates, ToolCallContext},
    db::{get_tool_by_name, Action, ToolDefinition},
};
use rusqlite::Connection;
use std::collections::HashMap;

/// Replace `{{param_name}}` placeholders in each action using the supplied argument map.
pub fn substitute_tool_args(actions: &[Action], args: &HashMap<String, String>) -> Vec<Action> {
    let ctx = ToolCallContext::from_args(args);
    actions
        .iter()
        .map(|action| resolve_action_templates(action, &[], Some(&ctx)))
        .collect()
}

/// Load a tool definition by machine id (`name` column).
pub fn load_tool(conn: &Connection, tool_id: &str) -> Result<ToolDefinition, String> {
    get_tool_by_name(conn, tool_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("tool `{tool_id}` was not found"))
}

/// Resolve args, then run the tool's action chain through the shared executor.
pub fn execute_tool(
    conn: &Connection,
    tool_id: &str,
    args: &HashMap<String, String>,
    runtime: &impl crate::commands::executor::ActionRuntime,
    app_index: Option<&[AppEntry]>,
) -> Result<(), String> {
    let tool = load_tool(conn, tool_id)?;
    if !tool.enabled {
        return Err(format!("tool `{tool_id}` is disabled"));
    }
    validate_tool_args(&tool, args)?;
    let resolved = substitute_tool_args(&tool.actions, args);
    execute_resolved_actions(&resolved, runtime, app_index);
    Ok(())
}

fn validate_tool_args(tool: &ToolDefinition, args: &HashMap<String, String>) -> Result<(), String> {
    for param in &tool.parameters {
        if !param.required {
            continue;
        }
        let value = args.get(&param.name).map(|s| s.trim()).unwrap_or("");
        if value.is_empty() {
            return Err(format!(
                "missing required tool argument `{}` for `{}`",
                param.name, tool.name
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::executor::ActionRuntime;
    use crate::db::{init_db, Action};
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    #[derive(Default, Clone)]
    struct MockState {
        url_calls: Vec<String>,
        statuses: Vec<String>,
        errors: Vec<String>,
    }

    #[derive(Clone, Default)]
    struct MockRuntime {
        state: Arc<Mutex<MockState>>,
    }

    impl ActionRuntime for MockRuntime {
        fn open_app(&self, _path: &str) -> Result<(), String> {
            Ok(())
        }

        fn open_url(&self, url: &str) -> Result<(), String> {
            self.state.lock().unwrap().url_calls.push(url.to_string());
            Ok(())
        }

        fn run_script(&self, _script: &str, _args: &[String]) -> Result<(), String> {
            Ok(())
        }

        fn send_keys(&self, _keys: &str) -> Result<(), String> {
            Ok(())
        }

        fn wait_ms(&self, _ms: u64) -> Result<(), String> {
            Ok(())
        }

        fn speak(&self, _text: &str) -> Result<(), String> {
            Ok(())
        }

        fn request_follow_up(&self, _prompt: &str) -> Result<String, String> {
            Err("not configured".into())
        }

        fn is_cancelled(&self) -> bool {
            false
        }

        fn emit_status(&self, text: &str) {
            self.state.lock().unwrap().statuses.push(text.to_string());
        }

        fn emit_error(&self, message: &str) {
            self.state.lock().unwrap().errors.push(message.to_string());
        }
    }

    fn test_conn() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tools-exec.db");
        init_db(&path).unwrap();
        (dir, Connection::open(&path).unwrap())
    }

    #[test]
    fn substitute_tool_args_replaces_url_placeholder() {
        let actions = vec![Action::OpenUrl {
            url: "{{url}}".into(),
        }];
        let args = [("url".to_string(), "https://example.com".to_string())]
            .into_iter()
            .collect();
        let resolved = substitute_tool_args(&actions, &args);
        assert_eq!(
            resolved,
            vec![Action::OpenUrl {
                url: "https://example.com".into()
            }]
        );
    }

    #[test]
    fn execute_tool_open_url_runs_builtin() {
        let (_dir, conn) = test_conn();
        let runtime = MockRuntime::default();
        let args = [("url".to_string(), "https://example.com".to_string())]
            .into_iter()
            .collect();
        execute_tool(&conn, "open_url", &args, &runtime, None).expect("execute open_url");
        let state = runtime.state.lock().unwrap();
        assert_eq!(state.url_calls, vec!["https://example.com".to_string()]);
        assert!(state.errors.is_empty());
    }

    #[test]
    fn execute_tool_rejects_missing_required_arg() {
        let (_dir, conn) = test_conn();
        let runtime = MockRuntime::default();
        let args = HashMap::new();
        let err = execute_tool(&conn, "open_url", &args, &runtime, None).unwrap_err();
        assert!(err.contains("missing required tool argument `url`"));
    }
}

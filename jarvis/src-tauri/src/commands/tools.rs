//! Tool registry execution: load definitions, substitute `{{param}}` templates, run actions.

use crate::{
    apps::AppEntry,
    commands::{
        execution_context::ExecutionContext,
        executor::{execute_resolved_actions, resolve_action_templates, ToolCallContext},
        open_target::execute_open_target_tool,
    },
    db::{
        get_registered_script, get_tool_by_name, verify_registered_script_file, Action,
        ToolDefinition,
    },
    window::{snap_foreground_window, DEFAULT_MONITOR},
};
use rusqlite::Connection;
use std::collections::HashMap;

/// Replace `{{param_name}}` placeholders in each action using the supplied argument map.
pub fn substitute_tool_args(actions: &[Action], args: &HashMap<String, String>) -> Vec<Action> {
    let ctx = ToolCallContext::from_args(args);
    let exec_ctx = ExecutionContext::from_tool_context(Some(&ctx));
    actions
        .iter()
        .map(|action| resolve_action_templates(action, &exec_ctx, Some(&ctx)))
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
    if tool_id == "open_target" {
        return execute_open_target_tool(conn, args, runtime, app_index);
    }
    if tool_id == "snap_window" {
        return execute_snap_window_tool(args, runtime);
    }
    if tool_id == "run_registered_script" {
        return execute_run_registered_script_tool(conn, args, runtime);
    }
    let resolved = substitute_tool_args(&tool.actions, args);
    reject_unsandboxed_run_script(&resolved)?;
    execute_resolved_actions(&resolved, runtime, app_index, Some(conn));
    Ok(())
}

/// Run an allowlisted script by id; args must match declared `arg_names`.
pub fn execute_registered_script(
    conn: &Connection,
    script_id: &str,
    args: &[String],
    runtime: &impl crate::commands::executor::ActionRuntime,
) -> Result<(), String> {
    let id = script_id.trim();
    if id.is_empty() {
        return Err("script id cannot be empty".to_string());
    }
    let registered = get_registered_script(conn, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("unknown registered script id `{id}`"))?;
    verify_registered_script_file(&registered)?;
    if args.len() != registered.arg_names.len() {
        return Err(format!(
            "script `{id}` expects {} argument(s), got {}",
            registered.arg_names.len(),
            args.len()
        ));
    }
    for arg in args {
        validate_script_arg(arg)?;
    }
    runtime.run_script(&registered.path, args)?;
    Ok(())
}

fn execute_run_registered_script_tool(
    conn: &Connection,
    args: &HashMap<String, String>,
    runtime: &impl crate::commands::executor::ActionRuntime,
) -> Result<(), String> {
    let script_id = args
        .get("script_id")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "missing required tool argument `script_id` for `run_registered_script`".to_string()
        })?;
    let script_args = parse_tool_script_args(args.get("args").map(String::as_str))?;
    runtime.emit_status(&format!("Running script {script_id}…"));
    execute_registered_script(conn, script_id, &script_args, runtime)?;
    runtime.emit_status(&format!("Ran registered script {script_id}"));
    Ok(())
}

fn parse_tool_script_args(raw: Option<&str>) -> Result<Vec<String>, String> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(vec![]);
    };
    let parsed: Vec<String> = serde_json::from_str(raw).map_err(|e| {
        format!("`args` must be a JSON array of strings for `run_registered_script`: {e}")
    })?;
    for arg in &parsed {
        validate_script_arg(arg)?;
    }
    Ok(parsed)
}

fn validate_script_arg(arg: &str) -> Result<(), String> {
    let trimmed = arg.trim();
    if trimmed.is_empty() {
        return Err("script args cannot contain empty values".to_string());
    }
    if trimmed.chars().any(is_shell_metachar) {
        return Err(format!(
            "script arg contains forbidden shell metacharacters: `{trimmed}`"
        ));
    }
    Ok(())
}

fn is_shell_metachar(c: char) -> bool {
    matches!(
        c,
        '&' | '|' | '<' | '>' | '^' | '%' | '!' | ';' | '`' | '"' | '\'' | '\n' | '\r'
    )
}

fn reject_unsandboxed_run_script(actions: &[Action]) -> Result<(), String> {
    for action in actions {
        if matches!(action, Action::RunScript { .. }) {
            return Err(
                "RunScript is not allowed in router-invoked tools; use run_registered_script"
                    .into(),
            );
        }
    }
    Ok(())
}

fn execute_snap_window_tool(
    args: &HashMap<String, String>,
    runtime: &impl crate::commands::executor::ActionRuntime,
) -> Result<(), String> {
    let zone = args
        .get("zone")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "missing required tool argument `zone` for `snap_window`".to_string())?;
    runtime.emit_status(&format!("Snapping window to {zone}…"));
    snap_foreground_window(zone, DEFAULT_MONITOR).inspect_err(|err| {
        runtime.emit_error(err);
    })?;
    runtime.emit_status(&format!("Snapped window to {zone}"));
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
    use crate::db::{hash_file, init_db, upsert_registered_script, Action, RegisteredScript};
    use rusqlite::Connection;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    #[derive(Default, Clone)]
    struct MockState {
        url_calls: Vec<String>,
        script_calls: Vec<(String, Vec<String>)>,
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

        fn run_script(&self, script: &str, args: &[String]) -> Result<(), String> {
            self.state
                .lock()
                .unwrap()
                .script_calls
                .push((script.to_string(), args.to_vec()));
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
    fn execute_tool_open_target_resolves_app() {
        let (_dir, conn) = test_conn();
        let runtime = MockRuntime::default();
        let index = vec![crate::apps::AppEntry {
            display_name: "Brave".into(),
            exe_path: r"C:\Brave\brave.exe".into(),
            icon_data_url: None,
        }];
        let args = [("target".to_string(), "brave".to_string())]
            .into_iter()
            .collect();
        execute_tool(&conn, "open_target", &args, &runtime, Some(&index))
            .expect("execute open_target");
        let state = runtime.state.lock().unwrap();
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

    #[test]
    fn execute_tool_rejects_run_script_in_user_tool() {
        let (_dir, conn) = test_conn();
        let runtime = MockRuntime::default();
        crate::db::insert_tool(
            &conn,
            &crate::db::NewToolDefinition {
                name: "unsafe_script".into(),
                display_name: "Unsafe".into(),
                description: "bad".into(),
                parameters: vec![crate::db::ToolParameter {
                    name: "path".into(),
                    param_type: "string".into(),
                    description: None,
                    required: true,
                    enum_values: vec![],
                }],
                actions: vec![Action::RunScript {
                    script: "{{path}}".into(),
                    args: vec![],
                }],
                enabled: true,
                builtin: false,
            },
        )
        .unwrap();
        let args = [("path".to_string(), "whoami".to_string())]
            .into_iter()
            .collect();
        let err = execute_tool(&conn, "unsafe_script", &args, &runtime, None).unwrap_err();
        assert!(err.contains("run_registered_script"));
    }

    #[test]
    fn run_registered_script_rejects_unknown_id() {
        let (_dir, conn) = test_conn();
        let runtime = MockRuntime::default();
        let err = execute_tool(
            &conn,
            "run_registered_script",
            &HashMap::from([("script_id".to_string(), "nope".to_string())]),
            &runtime,
            None,
        )
        .unwrap_err();
        assert!(err.contains("unknown registered script"));
        assert!(runtime.state.lock().unwrap().script_calls.is_empty());
    }

    #[test]
    fn run_registered_script_uses_allowlisted_path_only() {
        let (dir, conn) = test_conn();
        let script_path = dir.path().join("hello.cmd");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "@echo hello").unwrap();
        let hash = hash_file(&script_path).unwrap();
        upsert_registered_script(
            &conn,
            &RegisteredScript {
                id: "hello".into(),
                path: script_path.display().to_string(),
                hash,
                display_name: "Hello".into(),
                arg_names: vec![],
            },
        )
        .unwrap();
        let runtime = MockRuntime::default();
        execute_tool(
            &conn,
            "run_registered_script",
            &HashMap::from([("script_id".to_string(), "hello".to_string())]),
            &runtime,
            None,
        )
        .expect("run registered");
        let calls = &runtime.state.lock().unwrap().script_calls;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, script_path.display().to_string());
        assert!(calls[0].1.is_empty());
    }
}

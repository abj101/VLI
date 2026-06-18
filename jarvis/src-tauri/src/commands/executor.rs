use crate::{
    apps::AppEntry,
    audio::tts,
    commands::{
        execution_context::ExecutionContext,
        open_target::execute_open_target_with_aliases,
        values::Value,
    },
    db::{Action, CommandNode, IfConditionKind, TargetAlias},
    process::hidden_command,
    window::{
        focus_existing_app_window, place_window_after_app_launch, snap_foreground_window,
        snapshot_top_level_windows, DEFAULT_MONITOR,
    },
};
use log::debug;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use rusqlite::Connection;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

pub const ACTION_STATUS_EVENT: &str = "action-status";
pub const ACTION_ERROR_EVENT: &str = "action-error";
const ACTION_CANCELLED_MSG: &str = "Action run cancelled";
const MAX_RUN_DEPTH: usize = 8;

pub struct ActionOutcome {
    pub value: Value,
    pub status: String,
}

#[derive(Debug, Clone, Default)]
struct RunState {
    command_stack: Vec<i64>,
    depth: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestStepResult {
    pub index: usize,
    pub action_kind: String,
    pub status: String,
    pub output_preview: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestCommandResult {
    pub steps: Vec<TestStepResult>,
}

/// Optional tool-call argument map for `{{param_name}}` template substitution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCallContext {
    pub args: HashMap<String, String>,
}

impl ToolCallContext {
    #[allow(dead_code)]
    pub fn from_args(args: &HashMap<String, String>) -> Self {
        Self {
            args: args.clone(),
        }
    }

    pub fn with_remainder(remainder: &str) -> Self {
        let mut args = HashMap::new();
        args.insert("remainder".to_string(), remainder.to_string());
        Self { args }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionStatus {
    pub text: String,
}

pub trait ActionRuntime {
    fn open_app(&self, path: &str) -> Result<(), String>;
    fn open_url(&self, url: &str) -> Result<(), String>;
    fn run_script(&self, script: &str, args: &[String]) -> Result<(), String>;
    fn send_keys(&self, keys: &str) -> Result<(), String>;
    fn wait_ms(&self, ms: u64) -> Result<(), String>;
    fn speak(&self, text: &str) -> Result<(), String>;
    fn request_follow_up(&self, prompt: &str) -> Result<String, String>;
    fn is_cancelled(&self) -> bool;
    fn emit_status(&self, text: &str);
    fn emit_error(&self, message: &str);
    fn show_notification(&self, title: &str, body: &str) -> Result<(), String> {
        let _ = (title, body);
        Ok(())
    }
    fn persist_target_alias(&self, alias: &TargetAlias) -> Result<(), String> {
        let _ = alias;
        Ok(())
    }
    fn start_dictation(&self) -> Result<(), String> {
        Ok(())
    }
    fn stop_dictation(&self) -> Result<(), String> {
        Ok(())
    }
}

pub struct TauriActionRuntime<'a> {
    app: &'a AppHandle,
    cancel_flag: Option<Arc<AtomicBool>>,
    follow_up_handler: Option<Box<FollowUpHandler>>,
}

type FollowUpHandler = dyn Fn(&str) -> Result<String, String> + Send + Sync + 'static;

impl<'a> TauriActionRuntime<'a> {
    pub fn new(app: &'a AppHandle, cancel_flag: Arc<AtomicBool>) -> Self {
        Self {
            app,
            cancel_flag: Some(cancel_flag),
            follow_up_handler: None,
        }
    }

    pub fn with_follow_up_handler(
        app: &'a AppHandle,
        cancel_flag: Arc<AtomicBool>,
        follow_up_handler: Box<FollowUpHandler>,
    ) -> Self {
        let mut runtime = Self::new(app, cancel_flag);
        runtime.follow_up_handler = Some(follow_up_handler);
        runtime
    }
}

impl ActionRuntime for TauriActionRuntime<'_> {
    fn open_app(&self, path: &str) -> Result<(), String> {
        debug!("executor: open_app path={path:?}");
        let trimmed = path.trim();
        let status = if trimmed
            .to_ascii_lowercase()
            .starts_with("shell:appsfolder\\")
        {
            let windir = std::env::var("WINDIR")
                .or_else(|_| std::env::var("SystemRoot"))
                .unwrap_or_else(|_| "C:\\Windows".to_string());
            let explorer = Path::new(&windir).join("explorer.exe");
            hidden_command(explorer)
                .arg(trimmed)
                .status()
                .map_err(|e| format!("failed to launch app `{path}`: {e}"))?
        } else {
            hidden_command("cmd")
                .arg("/C")
                .arg("start")
                .arg("")
                .arg(trimmed)
                .status()
                .map_err(|e| format!("failed to launch app `{path}`: {e}"))?
        };
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "failed to launch app `{path}`: command exited with {status}"
            ))
        }
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        debug!("executor: open_url url={url:?}");
        self.app
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| format!("failed to open url `{url}`: {e}"))
    }

    fn run_script(&self, script: &str, args: &[String]) -> Result<(), String> {
        debug!("executor: run_script script={script:?} args={args:?}");
        let status = hidden_command(script)
            .args(args)
            .status()
            .map_err(|e| format!("failed to run script `{script}`: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "failed to run script `{script}`: command exited with {status}"
            ))
        }
    }

    fn send_keys(&self, keys: &str) -> Result<(), String> {
        debug!("executor: send_keys keys={keys:?}");
        let escaped = keys.replace('\'', "''");
        let command = format!(
            "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{escaped}')"
        );
        let status = hidden_command("powershell")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(command)
            .status()
            .map_err(|e| format!("failed to send keys `{keys}`: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "failed to send keys `{keys}`: powershell exited with {status}"
            ))
        }
    }

    fn wait_ms(&self, ms: u64) -> Result<(), String> {
        debug!("executor: wait_ms ms={ms}");
        let mut remaining = ms;
        while remaining > 0 {
            if self.is_cancelled() {
                return Err(ACTION_CANCELLED_MSG.to_string());
            }
            let chunk = remaining.min(50);
            thread::sleep(Duration::from_millis(chunk));
            remaining -= chunk;
        }
        Ok(())
    }

    fn speak(&self, text: &str) -> Result<(), String> {
        debug!("executor: speak chars={}", text.chars().count());
        tts::speak_with_piper(self.app, text)
    }

    fn request_follow_up(&self, prompt: &str) -> Result<String, String> {
        match &self.follow_up_handler {
            Some(handler) => handler(prompt),
            None => Err("SubPrompt follow-up handler is not configured".to_string()),
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel_flag
            .as_ref()
            .map(|f| f.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    fn emit_status(&self, text: &str) {
        let _ = self.app.emit(
            ACTION_STATUS_EVENT,
            ActionStatus {
                text: text.to_string(),
            },
        );
    }

    fn emit_error(&self, message: &str) {
        let _ = self.app.emit(
            ACTION_ERROR_EVENT,
            serde_json::json!({ "message": message }),
        );
    }

    fn show_notification(&self, title: &str, body: &str) -> Result<(), String> {
        let title = title.trim();
        let body = body.trim();
        let status = if title.is_empty() {
            body.to_string()
        } else if body.is_empty() {
            title.to_string()
        } else {
            format!("{title}: {body}")
        };
        self.emit_status(&format!("Notification — {status}"));
        Ok(())
    }

    fn start_dictation(&self) -> Result<(), String> {
        let hud = self.app.state::<crate::SharedHud>();
        let audio = self.app.state::<crate::SharedAudioPipeline>();
        crate::dictation::start_dictation(self.app, &*hud, &*audio)?;
        Ok(())
    }

    fn stop_dictation(&self) -> Result<(), String> {
        let hud = self.app.state::<crate::SharedHud>();
        let audio = self.app.state::<crate::SharedAudioPipeline>();
        crate::dictation::stop_dictation(self.app, &*hud, &*audio)
    }

    fn persist_target_alias(&self, alias: &TargetAlias) -> Result<(), String> {
        let conn = crate::open_db_connection(self.app)?;
        crate::db::upsert_target_alias(&conn, alias).map_err(|e| e.to_string())
    }
}

pub fn execute_command_with_context(
    node: &CommandNode,
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
    tool_context: Option<ToolCallContext>,
    target_aliases: Option<&[TargetAlias]>,
) {
    debug!(
        "executor: execute_command node_id={} name={:?} actions={}",
        node.id,
        node.name,
        node.actions.len()
    );
    execute_actions(
        &node.actions,
        runtime,
        app_index,
        tool_context.as_ref(),
        target_aliases,
        None,
        None,
        &mut ExecutionContext::from_tool_context(tool_context.as_ref()),
        &mut RunState::default(),
        None,
    );
    debug!("executor: execute_command finished node_id={}", node.id);
}

#[allow(dead_code)]
pub fn execute_resolved_actions(
    actions: &[Action],
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
    script_conn: Option<&Connection>,
) {
    execute_actions(
        actions,
        runtime,
        app_index,
        None,
        None,
        script_conn,
        None,
        &mut ExecutionContext::default(),
        &mut RunState::default(),
        None,
    );
}

/// Dry-run a command in the editor: same executor as voice, with per-step output capture.
pub fn test_command_actions(
    node: &CommandNode,
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
    test_input: Option<&str>,
    command_conn: Option<&Connection>,
) -> TestCommandResult {
    let tool_context = test_input.map(ToolCallContext::with_remainder);
    let mut steps = Vec::new();
    execute_actions(
        &node.actions,
        runtime,
        app_index,
        tool_context.as_ref(),
        None,
        None,
        command_conn,
        &mut ExecutionContext::from_tool_context(tool_context.as_ref()),
        &mut RunState::default(),
        Some(&mut steps),
    );
    TestCommandResult { steps }
}

fn execute_actions(
    actions: &[Action],
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
    tool_context: Option<&ToolCallContext>,
    target_aliases: Option<&[TargetAlias]>,
    script_conn: Option<&Connection>,
    command_conn: Option<&Connection>,
    exec_ctx: &mut ExecutionContext,
    run_state: &mut RunState,
    mut step_collector: Option<&mut Vec<TestStepResult>>,
) {
    for (step_index, action) in actions.iter().enumerate() {
        if runtime.is_cancelled() {
            runtime.emit_status(ACTION_CANCELLED_MSG);
            return;
        }
        let resolved = resolve_action_templates(action, exec_ctx, tool_context);
        let action_kind = action_kind_label(&resolved);
        let outcome = if let Action::TextCombine { separator } = &resolved {
            combine_text_value(separator, exec_ctx.last_result.as_ref())
        } else if let Action::IfElse {
            condition,
            text,
            pattern,
            then_actions,
            else_actions,
        } = &resolved
        {
            run_if_else_branch(
                *condition,
                text,
                pattern,
                then_actions,
                else_actions,
                runtime,
                app_index,
                tool_context,
                target_aliases,
                script_conn,
                command_conn,
                exec_ctx,
                run_state,
                step_collector.as_deref_mut(),
            )
        } else if let Action::RunCommand {
            command_id,
            input,
        } = &resolved
        {
            run_nested_command(
                *command_id,
                input.as_deref(),
                runtime,
                app_index,
                target_aliases,
                script_conn,
                command_conn,
                exec_ctx,
                run_state,
                step_collector.as_deref_mut(),
            )
        } else {
            execute_one_action(
                &resolved,
                runtime,
                app_index,
                target_aliases,
                script_conn,
            )
        };
        match outcome {
            Ok(outcome) => {
                runtime.emit_status(&outcome.status);
                exec_ctx.record_step_output(outcome.value.clone());
                if let Some(collector) = step_collector.as_deref_mut() {
                    collector.push(TestStepResult {
                        index: step_index,
                        action_kind: action_kind.to_string(),
                        status: outcome.status.clone(),
                        output_preview: outcome.value.display_string(),
                        error: None,
                    });
                }
            }
            Err(err) => {
                if err == ACTION_CANCELLED_MSG {
                    runtime.emit_status(ACTION_CANCELLED_MSG);
                    return;
                }
                let status = format!("Failed: {err}");
                runtime.emit_status(&status);
                runtime.emit_error(&err);
                if let Some(collector) = step_collector.as_deref_mut() {
                    collector.push(TestStepResult {
                        index: step_index,
                        action_kind: action_kind.to_string(),
                        status: status.clone(),
                        output_preview: String::new(),
                        error: Some(err.clone()),
                    });
                }
                if matches!(action, Action::SubPrompt { .. }) {
                    return;
                }
            }
        }
        if let Action::SubPrompt { prompt } = &resolved {
            if let Err(err) = runtime.speak(prompt) {
                runtime.emit_status(&format!(
                    "Follow-up prompt voice unavailable ({err}); showing text prompt"
                ));
            }
            match runtime.request_follow_up(prompt) {
                Ok(response) => {
                    exec_ctx.follow_up_responses.push(response.clone());
                    runtime.emit_status(&response);
                }
                Err(err) => {
                    if err == ACTION_CANCELLED_MSG {
                        runtime.emit_status(ACTION_CANCELLED_MSG);
                    } else {
                        runtime.emit_status(&format!("Failed: {err}"));
                        runtime.emit_error(&err);
                    }
                    return;
                }
            }
        }
    }
}

fn run_nested_command(
    command_id: i64,
    input: Option<&str>,
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
    target_aliases: Option<&[TargetAlias]>,
    script_conn: Option<&Connection>,
    command_conn: Option<&Connection>,
    parent_ctx: &ExecutionContext,
    run_state: &mut RunState,
    step_collector: Option<&mut Vec<TestStepResult>>,
) -> Result<ActionOutcome, String> {
    let conn = command_conn
        .ok_or_else(|| "RunCommand requires a database connection".to_string())?;
    if run_state.depth >= MAX_RUN_DEPTH {
        return Err(format!("RunCommand exceeded max depth of {MAX_RUN_DEPTH}"));
    }
    if run_state.command_stack.contains(&command_id) {
        return Err(format!("RunCommand cycle detected for command {command_id}"));
    }
    let nested = crate::db::get_command_by_id(conn, command_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("RunCommand target command {command_id} not found"))?;
    let nested_input = input
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            parent_ctx
                .last_result
                .as_ref()
                .map(|value| value.coerce_to_text())
        });
    let nested_tool_ctx = nested_input
        .as_ref()
        .map(|value| ToolCallContext::with_remainder(value));
    run_state.depth += 1;
    run_state.command_stack.push(command_id);
    let mut nested_ctx = ExecutionContext::from_tool_context(nested_tool_ctx.as_ref());
    execute_actions(
        &nested.actions,
        runtime,
        app_index,
        nested_tool_ctx.as_ref(),
        target_aliases,
        script_conn,
        command_conn,
        &mut nested_ctx,
        run_state,
        step_collector,
    );
    run_state.command_stack.pop();
    run_state.depth = run_state.depth.saturating_sub(1);
    let value = nested_ctx
        .last_result
        .clone()
        .unwrap_or(Value::Nothing);
    Ok(ActionOutcome {
        value: value.clone(),
        status: format!("Ran command {}", nested.name),
    })
}

fn run_if_else_branch(
    condition: IfConditionKind,
    text: &str,
    pattern: &str,
    then_actions: &[Action],
    else_actions: &[Action],
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
    tool_context: Option<&ToolCallContext>,
    target_aliases: Option<&[TargetAlias]>,
    script_conn: Option<&Connection>,
    command_conn: Option<&Connection>,
    exec_ctx: &mut ExecutionContext,
    run_state: &mut RunState,
    step_collector: Option<&mut Vec<TestStepResult>>,
) -> Result<ActionOutcome, String> {
    let pass = evaluate_if_condition(condition, text, pattern)?;
    let branch = if pass {
        then_actions
    } else {
        else_actions
    };
    let branch_label = if pass { "then" } else { "else" };
    let resolved_branch: Vec<Action> = branch
        .iter()
        .map(|action| resolve_action_templates(action, exec_ctx, tool_context))
        .collect();
    execute_actions(
        &resolved_branch,
        runtime,
        app_index,
        tool_context,
        target_aliases,
        script_conn,
        command_conn,
        exec_ctx,
        run_state,
        step_collector,
    );
    let value = exec_ctx
        .last_result
        .clone()
        .unwrap_or(Value::Nothing);
    Ok(ActionOutcome {
        value: value.clone(),
        status: format!("If/Else: {branch_label} branch"),
    })
}

fn action_kind_label(action: &Action) -> &'static str {
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

fn outcome_from_status(status: String) -> ActionOutcome {
    ActionOutcome {
        value: Value::Text(status.clone()),
        status,
    }
}

fn speak_outcome(text: &str) -> ActionOutcome {
    ActionOutcome {
        value: Value::Text(text.to_string()),
        status: format!("Spoke: {text}"),
    }
}

fn execute_one_action(
    action: &Action,
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
    target_aliases: Option<&[TargetAlias]>,
    script_conn: Option<&Connection>,
) -> Result<ActionOutcome, String> {
    match action {
        Action::OpenTarget { target, placement } => {
            let target_trimmed = target.trim();
            if target_trimmed.is_empty() {
                return Err("OpenTarget target cannot be empty".to_string());
            }
            let placement_ref = placement
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            execute_open_target_with_aliases(
                target_trimmed,
                placement_ref,
                target_aliases.unwrap_or(&[]),
                runtime,
                app_index,
            )
            .map(outcome_from_status)
        }
        Action::OpenApp {
            name,
            path,
            placement,
        } => {
            let trimmed = path.trim();
            let placement_ref = placement
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let (launch_path, display) = if trimmed.is_empty() {
                let n = name.trim();
                if n.is_empty() {
                    return Err("OpenApp path cannot be empty".to_string());
                }
                if let Some(entries) = app_index {
                    if let Some(hit) = crate::apps::resolve_app(n, entries) {
                        validate_open_app_path(&hit.exe_path)?;
                        (hit.exe_path.clone(), name.clone())
                    } else {
                        validate_open_app_start_fallback(n)?;
                        (n.to_string(), name.clone())
                    }
                } else {
                    validate_open_app_start_fallback(n)?;
                    (n.to_string(), name.clone())
                }
            } else {
                validate_open_app_path(trimmed)?;
                (trimmed.to_string(), name.clone())
            };
            if focus_existing_app_window(
                &launch_path,
                &display,
                placement_ref,
                DEFAULT_MONITOR,
            )? {
                return Ok(outcome_from_status(match placement_ref {
                    Some(zone) => format!("Focused {display} ({zone})..."),
                    None => format!("Focused {display}..."),
                }));
            }
            let snapshot = placement_ref.map(|_| snapshot_top_level_windows());
            runtime.open_app(&launch_path)?;
            if let (Some(zone), Some(before)) = (placement_ref, snapshot) {
                place_window_after_app_launch(&launch_path, zone, DEFAULT_MONITOR, &before)
                    .inspect_err(|err| {
                        runtime.emit_error(err);
                    })?;
                return Ok(outcome_from_status(format!("Opening {display} ({zone})...")));
            }
            Ok(outcome_from_status(format!("Opening {display}...")))
        }
        Action::PlaceWindow { zone, monitor } => {
            let zone = zone.trim();
            if zone.is_empty() {
                return Err("PlaceWindow zone cannot be empty".to_string());
            }
            let monitor = monitor
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(DEFAULT_MONITOR);
            snap_foreground_window(zone, monitor).inspect_err(|err| {
                runtime.emit_error(err);
            })?;
            Ok(outcome_from_status(format!("Snapped window to {zone}")))
        }
        Action::OpenUrl { url } => {
            validate_open_url(url)?;
            runtime.open_url(url)?;
            Ok(outcome_from_status(format!("Opening {url}...")))
        }
        Action::RunScript { script, args } => {
            validate_run_script(script, args)?;
            runtime.run_script(script, args)?;
            Ok(outcome_from_status(format!("Ran script {script}")))
        }
        Action::RunRegisteredScript { script_id, args } => {
            let conn = script_conn.ok_or_else(|| {
                "RunRegisteredScript requires a database connection (tool router path only)"
                    .to_string()
            })?;
            crate::commands::tools::execute_registered_script(conn, script_id, args, runtime)?;
            Ok(outcome_from_status(format!("Ran registered script {script_id}")))
        }
        Action::SendKeys { keys } => {
            validate_send_keys(keys)?;
            runtime.send_keys(keys)?;
            Ok(outcome_from_status("Sent key sequence".to_string()))
        }
        Action::Wait { ms } => {
            validate_wait_ms(*ms)?;
            runtime.wait_ms(*ms)?;
            Ok(outcome_from_status(format!("Waiting {ms}ms...")))
        }
        Action::Speak { text } => {
            validate_speak_text(text)?;
            runtime.speak(text)?;
            Ok(speak_outcome(text))
        }
        Action::SubPrompt { prompt } => {
            validate_sub_prompt(prompt)?;
            Ok(outcome_from_status("follow up".to_string()))
        }
        Action::ReadFile { path } => {
            let contents = read_file_contents(path)?;
            Ok(text_outcome(
                contents.clone(),
                format!("Read {} bytes from file", contents.len()),
            ))
        }
        Action::HttpGet { url } => {
            let body = http_get_body(url)?;
            Ok(text_outcome(
                body.clone(),
                format!("HTTP GET returned {} bytes", body.len()),
            ))
        }
        Action::GetClipboard {} => {
            let text = get_clipboard_text()?;
            Ok(text_outcome(
                text.clone(),
                format!("Read {} characters from clipboard", text.chars().count()),
            ))
        }
        Action::ShowNotification { title, body } => {
            validate_show_notification(title, body)?;
            runtime.show_notification(title, body)?;
            let preview = if title.trim().is_empty() {
                body.trim().to_string()
            } else {
                title.trim().to_string()
            };
            Ok(nothing_outcome(format!("Notification: {preview}")))
        }
        Action::TextTrim { text } => {
            let trimmed = trim_text(text)?;
            Ok(text_outcome(
                trimmed.clone(),
                format!("Trimmed to {} characters", trimmed.chars().count()),
            ))
        }
        Action::TextMatch { pattern, text } => {
            let matched = match_text(pattern, text)?;
            Ok(text_outcome(
                matched.clone(),
                format!("Matched {} characters", matched.chars().count()),
            ))
        }
        Action::TextSplit { delimiter, text } => {
            let segments = split_text(delimiter, text)?;
            let count = segments.len();
            Ok(list_outcome(
                segments,
                format!("Split into {count} segments"),
            ))
        }
        Action::TextCombine { separator: _ } => {
            Err("TextCombine is handled by execute_actions".to_string())
        }
        Action::SetClipboard { text } => {
            set_clipboard_text(text)?;
            let preview = text.trim().chars().take(40).collect::<String>();
            Ok(nothing_outcome(format!("Set clipboard: {preview}")))
        }
        Action::ListFolder { path } => {
            let entries = list_folder_entries(path)?;
            let count = entries.len();
            Ok(list_outcome(
                entries,
                format!("Listed {count} entries in folder"),
            ))
        }
        Action::WriteFile { path, content } => {
            write_file_contents(path, content)?;
            Ok(nothing_outcome(format!("Wrote {} bytes to file", content.len())))
        }
        Action::GetFileMetadata { path } => {
            let meta = file_metadata_map(path)?;
            let name = meta
                .get("name")
                .map(Value::coerce_to_text)
                .unwrap_or_default();
            Ok(dict_outcome(
                meta,
                format!("Metadata for {name}"),
            ))
        }
        Action::Screenshot { path } => {
            let saved = capture_screenshot(path.as_deref())?;
            let display = saved.to_string_lossy().into_owned();
            Ok(file_path_outcome(
                saved,
                format!("Screenshot saved to {display}"),
            ))
        }
        Action::DeviceInfo {} => {
            let info = device_info_map();
            Ok(dict_outcome(info, "Device info".to_string()))
        }
        Action::IfElse { .. } => Err("IfElse is handled by execute_actions".to_string()),
        Action::StartDictation {} => {
            runtime.start_dictation()?;
            Ok(nothing_outcome("Started dictation".into()))
        }
        Action::StopDictation {} => {
            runtime.stop_dictation()?;
            Ok(nothing_outcome("Stopped dictation".into()))
        }
        Action::RunCommand { .. } => {
            Err("RunCommand is handled by execute_actions".to_string())
        }
    }
}

pub fn resolve_action_templates(
    action: &Action,
    exec_ctx: &ExecutionContext,
    tool_context: Option<&ToolCallContext>,
) -> Action {
    let render = |input: &str| {
        let legacy = exec_ctx
            .follow_up_responses
            .last()
            .map(|response| input.replace("{{follow_up}}", response))
            .unwrap_or_else(|| input.to_string());
        let numbered = replace_numbered_variables(&legacy, &exec_ctx.follow_up_responses);
        let magic = apply_magic_variable_templates(&numbered, exec_ctx);
        apply_tool_arg_templates(&magic, tool_context)
    };
    let resolved = match action {
        Action::OpenApp {
            name,
            path,
            placement,
        } => Action::OpenApp {
            name: render(name),
            path: render(path),
            placement: placement.as_ref().map(|value| render(value)),
        },
        Action::PlaceWindow { zone, monitor } => Action::PlaceWindow {
            zone: render(zone),
            monitor: monitor.as_ref().map(|value| render(value)),
        },
        Action::OpenUrl { url } => Action::OpenUrl { url: render(url) },
        Action::OpenTarget { target, placement } => Action::OpenTarget {
            target: render(target),
            placement: placement.as_ref().map(|value| render(value)),
        },
        Action::RunScript { script, args } => Action::RunScript {
            script: render(script),
            args: args.iter().map(|arg| render(arg)).collect(),
        },
        Action::RunRegisteredScript { script_id, args } => Action::RunRegisteredScript {
            script_id: render(script_id),
            args: args.iter().map(|arg| render(arg)).collect(),
        },
        Action::SendKeys { keys } => Action::SendKeys { keys: render(keys) },
        Action::Wait { ms } => Action::Wait { ms: *ms },
        Action::Speak { text } => Action::Speak { text: render(text) },
        Action::SubPrompt { prompt } => Action::SubPrompt {
            prompt: render(prompt),
        },
        Action::RunCommand { command_id, input } => Action::RunCommand {
            command_id: *command_id,
            input: input.as_ref().map(|value| render(value)),
        },
        Action::ReadFile { path } => Action::ReadFile {
            path: render(path),
        },
        Action::HttpGet { url } => Action::HttpGet { url: render(url) },
        Action::GetClipboard {} => Action::GetClipboard {},
        Action::ShowNotification { title, body } => Action::ShowNotification {
            title: render(title),
            body: render(body),
        },
        Action::TextTrim { text } => Action::TextTrim { text: render(text) },
        Action::TextMatch { pattern, text } => Action::TextMatch {
            pattern: render(pattern),
            text: render(text),
        },
        Action::TextSplit { delimiter, text } => Action::TextSplit {
            delimiter: render(delimiter),
            text: render(text),
        },
        Action::TextCombine { separator } => Action::TextCombine {
            separator: render(separator),
        },
        Action::SetClipboard { text } => Action::SetClipboard { text: render(text) },
        Action::ListFolder { path } => Action::ListFolder {
            path: render(path),
        },
        Action::WriteFile { path, content } => Action::WriteFile {
            path: render(path),
            content: render(content),
        },
        Action::GetFileMetadata { path } => Action::GetFileMetadata {
            path: render(path),
        },
        Action::Screenshot { path } => Action::Screenshot {
            path: path.as_ref().map(|value| render(value)),
        },
        Action::DeviceInfo {} => Action::DeviceInfo {},
        Action::IfElse {
            condition,
            text,
            pattern,
            then_actions,
            else_actions,
        } => Action::IfElse {
            condition: *condition,
            text: render(text),
            pattern: render(pattern),
            then_actions: then_actions.clone(),
            else_actions: else_actions.clone(),
        },
        Action::StartDictation {} => Action::StartDictation {},
        Action::StopDictation {} => Action::StopDictation {},
    };
    apply_implicit_passthrough(&resolved, exec_ctx)
}

fn apply_magic_variable_templates(input: &str, exec_ctx: &ExecutionContext) -> String {
    let mut out = input.to_string();
    if let Some(value) = &exec_ctx.last_result {
        out = out.replace("{{last_result}}", &value.coerce_to_text());
    }
    if let Some(value) = &exec_ctx.shortcut_input {
        let text = value.coerce_to_text();
        out = out.replace("{{shortcut_input}}", &text);
        out = out.replace("{{remainder}}", &text);
    }
    out = replace_step_variable_templates(&out, exec_ctx);
    out
}

fn replace_step_variable_templates(input: &str, exec_ctx: &ExecutionContext) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < input.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some((end, index)) = match_step_variable(bytes, i) {
                let replacement = exec_ctx
                    .step_output_at(index)
                    .map(Value::coerce_to_text)
                    .unwrap_or_else(|| input[i..end].to_string());
                out.push_str(&replacement);
                i = end;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap_or_default();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn match_step_variable(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    const PREFIX: &[u8] = b"{{step_";
    if start + PREFIX.len() > bytes.len() || &bytes[start..start + PREFIX.len()] != PREFIX {
        return None;
    }
    let mut idx = start + PREFIX.len();
    let digits_start = idx;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if digits_start == idx || idx + 2 > bytes.len() || &bytes[idx..idx + 2] != b"}}" {
        return None;
    }
    let index = std::str::from_utf8(&bytes[digits_start..idx])
        .ok()?
        .parse::<usize>()
        .ok()?;
    if index == 0 {
        return None;
    }
    Some((idx + 2, index))
}

fn apply_implicit_passthrough(action: &Action, exec_ctx: &ExecutionContext) -> Action {
    let passthrough = || {
        exec_ctx
            .last_result
            .as_ref()
            .map(|value| value.coerce_to_text())
    };
    match action {
        Action::OpenUrl { url } if url.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::OpenUrl { url: text }
            } else {
                action.clone()
            }
        }
        Action::Speak { text } if text.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::Speak { text }
            } else {
                action.clone()
            }
        }
        Action::OpenTarget { target, placement } if target.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::OpenTarget {
                    target: text,
                    placement: placement.clone(),
                }
            } else {
                action.clone()
            }
        }
        Action::ReadFile { path } if path.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::ReadFile { path: text }
            } else {
                action.clone()
            }
        }
        Action::HttpGet { url } if url.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::HttpGet { url: text }
            } else {
                action.clone()
            }
        }
        Action::ShowNotification { title, body } if body.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::ShowNotification {
                    title: title.clone(),
                    body: text,
                }
            } else {
                action.clone()
            }
        }
        Action::TextTrim { text } if text.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::TextTrim { text }
            } else {
                action.clone()
            }
        }
        Action::TextMatch { pattern, text } if text.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::TextMatch {
                    pattern: pattern.clone(),
                    text,
                }
            } else {
                action.clone()
            }
        }
        Action::TextSplit { delimiter, text } if text.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::TextSplit {
                    delimiter: delimiter.clone(),
                    text,
                }
            } else {
                action.clone()
            }
        }
        Action::SetClipboard { text } if text.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::SetClipboard { text }
            } else {
                action.clone()
            }
        }
        Action::ListFolder { path } if path.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::ListFolder { path: text }
            } else {
                action.clone()
            }
        }
        Action::WriteFile { path, content } if path.trim().is_empty() && content.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::WriteFile {
                    path: text,
                    content: String::new(),
                }
            } else {
                action.clone()
            }
        }
        Action::WriteFile { path, content } if content.trim().is_empty() && !path.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::WriteFile {
                    path: path.clone(),
                    content: text,
                }
            } else {
                action.clone()
            }
        }
        Action::WriteFile { path, content } if path.trim().is_empty() && !content.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::WriteFile {
                    path: text,
                    content: content.clone(),
                }
            } else {
                action.clone()
            }
        }
        Action::GetFileMetadata { path } if path.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::GetFileMetadata { path: text }
            } else {
                action.clone()
            }
        }
        Action::IfElse {
            condition,
            text,
            pattern,
            then_actions,
            else_actions,
        } if text.trim().is_empty() => {
            if let Some(text) = passthrough() {
                Action::IfElse {
                    condition: *condition,
                    text,
                    pattern: pattern.clone(),
                    then_actions: then_actions.clone(),
                    else_actions: else_actions.clone(),
                }
            } else {
                action.clone()
            }
        }
        _ => action.clone(),
    }
}

fn apply_tool_arg_templates(input: &str, tool_context: Option<&ToolCallContext>) -> String {
    let Some(ctx) = tool_context else {
        return input.to_string();
    };
    let mut out = input.to_string();
    for (key, value) in &ctx.args {
        let placeholder = format!("{{{{{key}}}}}");
        out = out.replace(&placeholder, value);
    }
    out
}

fn replace_numbered_variables(input: &str, follow_up_responses: &[String]) -> String {
    if follow_up_responses.is_empty() {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < input.len() {
        if let Some((end, index)) = match_numbered_variable(bytes, i) {
            if let Some(value) = follow_up_responses.get(index.saturating_sub(1)) {
                out.push_str(value);
            } else {
                out.push_str(&input[i..end]);
            }
            i = end;
            continue;
        }
        let ch = input[i..].chars().next().unwrap_or_default();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn match_numbered_variable(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    const PREFIX: &str = "variable";
    if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
        return None;
    }
    if start + PREFIX.len() > bytes.len()
        || !std::str::from_utf8(&bytes[start..start + PREFIX.len()])
            .ok()?
            .eq_ignore_ascii_case(PREFIX)
    {
        return None;
    }
    let mut idx = start + PREFIX.len();
    if idx >= bytes.len() || !bytes[idx].is_ascii_whitespace() {
        return None;
    }
    while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
        idx += 1;
    }
    let digits_start = idx;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if digits_start == idx {
        return None;
    }
    if idx < bytes.len() && bytes[idx].is_ascii_alphanumeric() {
        return None;
    }
    let index = std::str::from_utf8(&bytes[digits_start..idx])
        .ok()?
        .parse::<usize>()
        .ok()?;
    if index == 0 {
        return None;
    }
    Some((idx, index))
}

fn validate_open_app_path(path: &str) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("OpenApp path cannot be empty".to_string());
    }
    let uwp_shell = trimmed
        .to_ascii_lowercase()
        .starts_with("shell:appsfolder\\");
    if trimmed.chars().any(|c| {
        if uwp_shell && c == '!' {
            return false;
        }
        is_shell_metachar(c)
    }) {
        return Err(format!(
            "OpenApp path contains forbidden shell metacharacters: `{trimmed}`"
        ));
    }
    Ok(())
}

fn validate_open_app_start_fallback(name: &str) -> Result<(), String> {
    if name.chars().any(is_shell_metachar) {
        return Err(format!(
            "OpenApp name contains forbidden shell metacharacters: `{name}`"
        ));
    }
    Ok(())
}

fn validate_open_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(())
    } else {
        Err(format!(
            "OpenUrl only supports http:// or https:// URLs: `{trimmed}`"
        ))
    }
}

fn validate_run_script(script: &str, args: &[String]) -> Result<(), String> {
    let trimmed = script.trim();
    if trimmed.is_empty() {
        return Err("RunScript script cannot be empty".to_string());
    }
    if trimmed.chars().any(is_shell_metachar) {
        return Err(format!(
            "RunScript script contains forbidden shell metacharacters: `{trimmed}`"
        ));
    }
    for arg in args {
        let arg_trimmed = arg.trim();
        if arg_trimmed.is_empty() {
            return Err("RunScript args cannot contain empty values".to_string());
        }
        if arg_trimmed.chars().any(is_shell_metachar) {
            return Err(format!(
                "RunScript arg contains forbidden shell metacharacters: `{arg_trimmed}`"
            ));
        }
    }
    Ok(())
}

fn validate_send_keys(keys: &str) -> Result<(), String> {
    let trimmed = keys.trim();
    if trimmed.is_empty() {
        return Err("SendKeys keys cannot be empty".to_string());
    }
    if trimmed.len() > 128 {
        return Err("SendKeys keys exceeds max length of 128".to_string());
    }
    if trimmed.chars().any(is_shell_metachar) {
        return Err(format!(
            "SendKeys contains forbidden shell metacharacters: `{trimmed}`"
        ));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("SendKeys cannot contain control characters".to_string());
    }
    Ok(())
}

fn validate_wait_ms(ms: u64) -> Result<(), String> {
    if ms == 0 {
        return Err("Wait duration must be greater than 0ms".to_string());
    }
    if ms > 60_000 {
        return Err("Wait duration exceeds max of 60000ms".to_string());
    }
    Ok(())
}

fn validate_speak_text(text: &str) -> Result<(), String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Speak text cannot be empty".to_string());
    }
    if trimmed.chars().count() > 400 {
        return Err("Speak text exceeds max length of 400 characters".to_string());
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("Speak text cannot contain control characters".to_string());
    }
    Ok(())
}

fn validate_sub_prompt(prompt: &str) -> Result<(), String> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return Err("SubPrompt prompt cannot be empty".to_string());
    }
    if trimmed.chars().count() > 200 {
        return Err("SubPrompt prompt exceeds max length of 200 characters".to_string());
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("SubPrompt prompt cannot contain control characters".to_string());
    }
    Ok(())
}

const MAX_READ_FILE_BYTES: u64 = 10 * 1024 * 1024;

fn validate_read_file_path(path: &str) -> Result<&str, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("ReadFile path cannot be empty".to_string());
    }
    Ok(trimmed)
}

pub fn read_file_contents(path: &str) -> Result<String, String> {
    let path = validate_read_file_path(path)?;
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("ReadFile failed to stat `{path}`: {e}"))?;
    if !meta.is_file() {
        return Err(format!("ReadFile path is not a file: `{path}`"));
    }
    if meta.len() > MAX_READ_FILE_BYTES {
        return Err(format!(
            "ReadFile exceeds max size of {MAX_READ_FILE_BYTES} bytes"
        ));
    }
    std::fs::read_to_string(path).map_err(|e| format!("ReadFile failed to read `{path}`: {e}"))
}

fn validate_http_get_url(url: &str) -> Result<&str, String> {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(trimmed)
    } else {
        Err(format!(
            "HttpGet only supports http:// or https:// URLs: `{trimmed}`"
        ))
    }
}

pub fn http_get_body(url: &str) -> Result<String, String> {
    let url = validate_http_get_url(url)?;
    let response = ureq::get(url)
        .call()
        .map_err(|e| format!("HTTP GET failed for `{url}`: {e}"))?;
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(format!("HTTP GET `{url}` returned status {status}"));
    }
    response
        .into_string()
        .map_err(|e| format!("HTTP GET body read failed for `{url}`: {e}"))
}

pub fn get_clipboard_text() -> Result<String, String> {
    arboard::Clipboard::new()
        .map_err(|e| format!("GetClipboard failed to access clipboard: {e}"))?
        .get_text()
        .map_err(|e| format!("GetClipboard failed to read text: {e}"))
}

fn validate_show_notification(title: &str, body: &str) -> Result<(), String> {
    if title.trim().is_empty() && body.trim().is_empty() {
        return Err("ShowNotification requires a title or body".to_string());
    }
    Ok(())
}

pub fn trim_text(text: &str) -> Result<String, String> {
    Ok(text.trim().to_string())
}

pub fn match_text(pattern: &str, text: &str) -> Result<String, String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Err("TextMatch pattern cannot be empty".to_string());
    }
    let re = regex::Regex::new(pattern)
        .map_err(|e| format!("TextMatch invalid regex `{pattern}`: {e}"))?;
    let caps = re
        .captures(text)
        .ok_or_else(|| format!("TextMatch found no match for `{pattern}`"))?;
    if let Some(group) = caps.get(1) {
        Ok(group.as_str().to_string())
    } else if let Some(full) = caps.get(0) {
        Ok(full.as_str().to_string())
    } else {
        Err(format!("TextMatch found no match for `{pattern}`"))
    }
}

pub fn split_text(delimiter: &str, text: &str) -> Result<Vec<String>, String> {
    if delimiter.is_empty() {
        return Err("TextSplit delimiter cannot be empty".to_string());
    }
    Ok(text.split(delimiter).map(str::to_string).collect())
}

pub fn combine_text_value(separator: &str, input: Option<&Value>) -> Result<ActionOutcome, String> {
    let Some(value) = input else {
        return Err("TextCombine requires output from a prior step".to_string());
    };
    let combined = match value {
        Value::List(items) => items
            .iter()
            .map(Value::coerce_to_text)
            .collect::<Vec<_>>()
            .join(separator),
        other => other.coerce_to_text(),
    };
    Ok(text_outcome(
        combined.clone(),
        format!("Combined into {} characters", combined.chars().count()),
    ))
}

pub fn set_clipboard_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("SetClipboard text cannot be empty".to_string());
    }
    arboard::Clipboard::new()
        .map_err(|e| format!("SetClipboard failed to access clipboard: {e}"))?
        .set_text(text)
        .map_err(|e| format!("SetClipboard failed to write text: {e}"))
}

fn validate_folder_path(path: &str) -> Result<&str, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("ListFolder path cannot be empty".to_string());
    }
    Ok(trimmed)
}

pub fn list_folder_entries(path: &str) -> Result<Vec<String>, String> {
    let path = validate_folder_path(path)?;
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("ListFolder failed to stat `{path}`: {e}"))?;
    if !meta.is_dir() {
        return Err(format!("ListFolder path is not a directory: `{path}`"));
    }
    let mut names = std::fs::read_dir(path)
        .map_err(|e| format!("ListFolder failed to read `{path}`: {e}"))?
        .filter_map(|entry| {
            entry
                .ok()
                .and_then(|e| e.file_name().into_string().ok())
        })
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

fn validate_write_file_path(path: &str) -> Result<&str, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("WriteFile path cannot be empty".to_string());
    }
    Ok(trimmed)
}

pub fn write_file_contents(path: &str, content: &str) -> Result<(), String> {
    let path = validate_write_file_path(path)?;
    if content.len() as u64 > MAX_READ_FILE_BYTES {
        return Err(format!(
            "WriteFile exceeds max size of {MAX_READ_FILE_BYTES} bytes"
        ));
    }
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("WriteFile failed to create parent dir for `{path}`: {e}"))?;
        }
    }
    std::fs::write(path, content.as_bytes())
        .map_err(|e| format!("WriteFile failed to write `{path}`: {e}"))
}

fn validate_metadata_path(path: &str) -> Result<&str, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("GetFileMetadata path cannot be empty".to_string());
    }
    Ok(trimmed)
}

pub fn file_metadata_map(path: &str) -> Result<BTreeMap<String, Value>, String> {
    let path = validate_metadata_path(path)?;
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("GetFileMetadata failed to stat `{path}`: {e}"))?;
    let path_buf = PathBuf::from(path);
    let name = path_buf
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let mut map = BTreeMap::new();
    map.insert("name".into(), Value::Text(name));
    map.insert("path".into(), Value::Text(path.to_string()));
    map.insert("size_bytes".into(), Value::Number(meta.len() as f64));
    map.insert("is_directory".into(), Value::Bool(meta.is_dir()));
    if let Ok(modified) = meta.modified() {
        if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
            map.insert(
                "modified_unix".into(),
                Value::Number(duration.as_secs_f64()),
            );
        }
    }
    Ok(map)
}

pub fn device_info_map() -> BTreeMap<String, Value> {
    use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
    let mut system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );
    system.refresh_cpu_all();
    system.refresh_memory();
    let cpu = system.global_cpu_usage() as f64;
    let cpu = if cpu.is_finite() { cpu } else { 0.0 };
    let mut map = BTreeMap::new();
    map.insert("cpu_usage_pct".into(), Value::Number(cpu));
    map.insert(
        "total_memory_bytes".into(),
        Value::Number(system.total_memory() as f64),
    );
    map.insert(
        "used_memory_bytes".into(),
        Value::Number(system.used_memory() as f64),
    );
    map.insert(
        "available_memory_bytes".into(),
        Value::Number(system.available_memory() as f64),
    );
    map
}

pub fn capture_screenshot(path: Option<&str>) -> Result<PathBuf, String> {
    use screenshots::Screen;
    let screens = Screen::all().map_err(|e| format!("Screenshot failed to list displays: {e}"))?;
    let screen = screens
        .into_iter()
        .next()
        .ok_or_else(|| "Screenshot found no displays".to_string())?;
    let image = screen
        .capture()
        .map_err(|e| format!("Screenshot capture failed: {e}"))?;
    let out_path = if let Some(path) = path.map(str::trim).filter(|p| !p.is_empty()) {
        PathBuf::from(path)
    } else {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "jarvis-screenshot-{}.png",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));
        dir
    };
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("Screenshot failed to create output dir `{}`: {e}", parent.display())
            })?;
        }
    }
    image
        .save(&out_path)
        .map_err(|e| format!("Screenshot failed to save `{}`: {e}", out_path.display()))?;
    Ok(out_path)
}

pub fn evaluate_if_condition(
    condition: IfConditionKind,
    text: &str,
    pattern: &str,
) -> Result<bool, String> {
    match condition {
        IfConditionKind::TextContains => {
            let needle = pattern.trim();
            if needle.is_empty() {
                return Err("IfElse text_contains requires a non-empty pattern".to_string());
            }
            Ok(text.contains(needle))
        }
        IfConditionKind::RegexMatch => {
            let pat = pattern.trim();
            if pat.is_empty() {
                return Err("IfElse regex_match requires a non-empty pattern".to_string());
            }
            let re = regex::Regex::new(pat)
                .map_err(|e| format!("IfElse invalid regex `{pat}`: {e}"))?;
            Ok(re.is_match(text))
        }
        IfConditionKind::TextIsEmpty => Ok(text.trim().is_empty()),
    }
}

fn nothing_outcome(status: String) -> ActionOutcome {
    ActionOutcome {
        value: Value::Nothing,
        status,
    }
}

fn text_outcome(text: String, status: String) -> ActionOutcome {
    ActionOutcome {
        value: Value::Text(text),
        status,
    }
}

fn list_outcome(segments: Vec<String>, status: String) -> ActionOutcome {
    ActionOutcome {
        value: Value::List(segments.into_iter().map(Value::Text).collect()),
        status,
    }
}

fn dict_outcome(map: BTreeMap<String, Value>, status: String) -> ActionOutcome {
    ActionOutcome {
        value: Value::Dict(map),
        status,
    }
}

fn file_path_outcome(path: PathBuf, status: String) -> ActionOutcome {
    ActionOutcome {
        value: Value::FilePath(path),
        status,
    }
}

fn is_shell_metachar(c: char) -> bool {
    matches!(
        c,
        '&' | '|' | '<' | '>' | '^' | '%' | '!' | ';' | '`' | '"' | '\'' | '\n' | '\r'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::AppEntry;
    use std::sync::{Arc, Mutex};

    fn node_with_actions(actions: Vec<Action>) -> CommandNode {
        CommandNode {
            id: 1,
            name: "test".into(),
            trigger_phrases: vec!["test".into()],
            actions,
            enabled: true,
            fuzzy_threshold_pct: 80,
            match_mode: crate::db::MatchMode::Phrase,
            created_at: "now".into(),
        }
    }

    #[derive(Default, Debug)]
    struct MockState {
        app_calls: Vec<String>,
        url_calls: Vec<String>,
        script_calls: Vec<(String, Vec<String>)>,
        key_calls: Vec<String>,
        wait_calls: Vec<u64>,
        speak_calls: Vec<String>,
        notification_calls: Vec<(String, String)>,
        statuses: Vec<String>,
        errors: Vec<String>,
        fail_app_paths: Vec<String>,
        fail_urls: Vec<String>,
        fail_scripts: Vec<String>,
        fail_keys: Vec<String>,
        fail_speak_texts: Vec<String>,
        follow_up_answers: Vec<String>,
        follow_up_prompts: Vec<String>,
        fail_follow_up_prompt: Option<String>,
        dictation_starts: usize,
        dictation_stops: usize,
        cancelled: bool,
    }

    #[derive(Clone, Default, Debug)]
    struct MockRuntime {
        state: Arc<Mutex<MockState>>,
    }

    impl MockRuntime {
        fn with_failures(fail_app_paths: Vec<&str>, fail_urls: Vec<&str>) -> Self {
            let mut s = MockState::default();
            s.fail_app_paths = fail_app_paths.into_iter().map(str::to_string).collect();
            s.fail_urls = fail_urls.into_iter().map(str::to_string).collect();
            Self {
                state: Arc::new(Mutex::new(s)),
            }
        }

        fn with_action_failures(fail_scripts: Vec<&str>, fail_keys: Vec<&str>) -> Self {
            let mut s = MockState::default();
            s.fail_scripts = fail_scripts.into_iter().map(str::to_string).collect();
            s.fail_keys = fail_keys.into_iter().map(str::to_string).collect();
            Self {
                state: Arc::new(Mutex::new(s)),
            }
        }

        fn snapshot(&self) -> MockState {
            self.state.lock().unwrap().clone()
        }

        fn with_cancelled() -> Self {
            let mut s = MockState::default();
            s.cancelled = true;
            Self {
                state: Arc::new(Mutex::new(s)),
            }
        }

        fn with_follow_up_answers(answers: Vec<&str>) -> Self {
            let mut s = MockState::default();
            s.follow_up_answers = answers.into_iter().map(str::to_string).collect();
            Self {
                state: Arc::new(Mutex::new(s)),
            }
        }

        fn with_follow_up_failure(prompt: &str) -> Self {
            let mut s = MockState::default();
            s.fail_follow_up_prompt = Some(prompt.to_string());
            Self {
                state: Arc::new(Mutex::new(s)),
            }
        }
    }

    impl Clone for MockState {
        fn clone(&self) -> Self {
            Self {
                app_calls: self.app_calls.clone(),
                url_calls: self.url_calls.clone(),
                script_calls: self.script_calls.clone(),
                key_calls: self.key_calls.clone(),
                wait_calls: self.wait_calls.clone(),
                speak_calls: self.speak_calls.clone(),
                notification_calls: self.notification_calls.clone(),
                statuses: self.statuses.clone(),
                errors: self.errors.clone(),
                fail_app_paths: self.fail_app_paths.clone(),
                fail_urls: self.fail_urls.clone(),
                fail_scripts: self.fail_scripts.clone(),
                fail_keys: self.fail_keys.clone(),
                fail_speak_texts: self.fail_speak_texts.clone(),
                follow_up_answers: self.follow_up_answers.clone(),
                follow_up_prompts: self.follow_up_prompts.clone(),
                fail_follow_up_prompt: self.fail_follow_up_prompt.clone(),
                dictation_starts: self.dictation_starts,
                dictation_stops: self.dictation_stops,
                cancelled: self.cancelled,
            }
        }
    }

    impl ActionRuntime for MockRuntime {
        fn open_app(&self, path: &str) -> Result<(), String> {
            let mut s = self.state.lock().unwrap();
            s.app_calls.push(path.to_string());
            if s.fail_app_paths.iter().any(|p| p == path) {
                return Err(format!("mock app launch failed: {path}"));
            }
            Ok(())
        }

        fn open_url(&self, url: &str) -> Result<(), String> {
            let mut s = self.state.lock().unwrap();
            s.url_calls.push(url.to_string());
            if s.fail_urls.iter().any(|u| u == url) {
                return Err(format!("mock url open failed: {url}"));
            }
            Ok(())
        }

        fn run_script(&self, script: &str, args: &[String]) -> Result<(), String> {
            let mut s = self.state.lock().unwrap();
            s.script_calls.push((script.to_string(), args.to_vec()));
            if s.fail_scripts.iter().any(|p| p == script) {
                return Err(format!("mock script failed: {script}"));
            }
            Ok(())
        }

        fn send_keys(&self, keys: &str) -> Result<(), String> {
            let mut s = self.state.lock().unwrap();
            s.key_calls.push(keys.to_string());
            if s.fail_keys.iter().any(|k| k == keys) {
                return Err(format!("mock send_keys failed: {keys}"));
            }
            Ok(())
        }

        fn wait_ms(&self, ms: u64) -> Result<(), String> {
            self.state.lock().unwrap().wait_calls.push(ms);
            Ok(())
        }

        fn speak(&self, text: &str) -> Result<(), String> {
            let mut s = self.state.lock().unwrap();
            s.speak_calls.push(text.to_string());
            if s.fail_speak_texts.iter().any(|t| t == text) {
                return Err(format!("mock speak failed: {text}"));
            }
            Ok(())
        }

        fn show_notification(&self, title: &str, body: &str) -> Result<(), String> {
            self.state
                .lock()
                .unwrap()
                .notification_calls
                .push((title.to_string(), body.to_string()));
            Ok(())
        }

        fn request_follow_up(&self, prompt: &str) -> Result<String, String> {
            let mut s = self.state.lock().unwrap();
            s.follow_up_prompts.push(prompt.to_string());
            if s.fail_follow_up_prompt.as_deref() == Some(prompt) {
                return Err("Follow-up timed out".to_string());
            }
            if s.follow_up_answers.is_empty() {
                return Err("Follow-up input not provided".to_string());
            }
            Ok(s.follow_up_answers.remove(0))
        }

        fn is_cancelled(&self) -> bool {
            self.state.lock().unwrap().cancelled
        }

        fn emit_status(&self, text: &str) {
            self.state.lock().unwrap().statuses.push(text.to_string());
        }

        fn emit_error(&self, message: &str) {
            self.state.lock().unwrap().errors.push(message.to_string());
        }

        fn start_dictation(&self) -> Result<(), String> {
            self.state.lock().unwrap().dictation_starts += 1;
            Ok(())
        }

        fn stop_dictation(&self) -> Result<(), String> {
            self.state.lock().unwrap().dictation_stops += 1;
            Ok(())
        }
    }

    #[test]
    fn executor_start_dictation_action() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::StartDictation {},
            Action::StopDictation {},
        ]);
        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.dictation_starts, 1);
        assert_eq!(s.dictation_stops, 1);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn open_app_empty_path_resolves_from_index() {
        let runtime = MockRuntime::default();
        let index = vec![AppEntry {
            display_name: "Calculator".into(),
            exe_path: "calc.exe".into(),
            icon_data_url: None,
        }];
        let node = node_with_actions(vec![Action::OpenApp {
            name: "calc".into(),
            path: "".into(),
            placement: None,
        }]);
        execute_command_with_context(&node, &runtime, Some(&index), None, None);
        let s = runtime.snapshot();
        assert_eq!(s.app_calls, vec!["calc.exe".to_string()]);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn open_app_empty_path_falls_back_to_name_when_unresolved() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![Action::OpenApp {
            name: "notepad".into(),
            path: "   ".into(),
            placement: None,
        }]);
        execute_command_with_context(&node, &runtime, Some(&[]), None, None);
        let s = runtime.snapshot();
        assert_eq!(s.app_calls, vec!["notepad".to_string()]);
    }

    #[test]
    fn validate_open_app_path_allows_shell_apps_folder_with_bang() {
        let p = "shell:AppsFolder\\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App";
        validate_open_app_path(p).expect("UWP shell path should validate");
    }

    #[test]
    fn rejects_shell_metacharacters_in_open_app_path() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![Action::OpenApp {
            name: "calc".into(),
            path: "calc.exe & whoami".into(),
            placement: None,
        }]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert!(s.app_calls.is_empty());
        assert_eq!(s.statuses.len(), 1);
        assert_eq!(s.errors.len(), 1);
        assert!(s.errors[0].contains("forbidden shell metacharacters"));
    }

    #[test]
    fn rejects_non_http_urls() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![Action::OpenUrl {
            url: "file:///etc/passwd".into(),
        }]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert!(s.url_calls.is_empty());
        assert_eq!(s.statuses.len(), 1);
        assert_eq!(s.errors.len(), 1);
        assert!(s.errors[0].contains("only supports http:// or https://"));
    }

    #[test]
    fn continues_remaining_actions_after_error() {
        let runtime = MockRuntime::with_failures(vec!["notepad.exe"], vec![]);
        let node = node_with_actions(vec![
            Action::OpenApp {
                name: "notepad".into(),
                path: "notepad.exe".into(),
                placement: None,
            },
            Action::OpenUrl {
                url: "https://github.com".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.app_calls, vec!["notepad.exe".to_string()]);
        assert_eq!(s.url_calls, vec!["https://github.com".to_string()]);
        assert_eq!(s.statuses.len(), 2);
        assert_eq!(s.errors.len(), 1);
    }

    #[test]
    fn emits_action_status_for_successful_actions() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::OpenApp {
                name: "notepad".into(),
                path: "notepad.exe".into(),
                placement: None,
            },
            Action::OpenUrl {
                url: "https://github.com".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(
            s.statuses,
            vec![
                "Opening notepad...".to_string(),
                "Opening https://github.com...".to_string(),
            ]
        );
        assert!(s.errors.is_empty());
    }

    #[test]
    fn executes_phase2_non_interactive_actions_in_declared_order() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::RunScript {
                script: "echo".into(),
                args: vec!["hello".into()],
            },
            Action::Wait { ms: 250 },
            Action::SendKeys {
                keys: "CTRL+SHIFT+N".into(),
            },
            Action::OpenUrl {
                url: "https://example.com".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(
            s.script_calls,
            vec![("echo".to_string(), vec!["hello".to_string()])]
        );
        assert_eq!(s.wait_calls, vec![250]);
        assert_eq!(s.key_calls, vec!["CTRL+SHIFT+N".to_string()]);
        assert_eq!(s.url_calls, vec!["https://example.com".to_string()]);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn rejects_unsafe_run_script_payloads() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![Action::RunScript {
            script: "whoami && del C:\\temp\\*".into(),
            args: vec![],
        }]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert!(s.script_calls.is_empty());
        assert_eq!(s.errors.len(), 1);
        assert!(s.errors[0].contains("RunScript script contains forbidden shell metacharacters"));
    }

    #[test]
    fn rejects_unsafe_send_keys_payloads() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![Action::SendKeys {
            keys: "CTRL+ALT+DEL;shutdown".into(),
        }]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert!(s.key_calls.is_empty());
        assert_eq!(s.errors.len(), 1);
        assert!(s.errors[0].contains("SendKeys contains forbidden shell metacharacters"));
    }

    #[test]
    fn wait_action_reports_status_and_allows_chain_to_continue() {
        let runtime = MockRuntime::with_action_failures(vec![], vec![]);
        let node = node_with_actions(vec![
            Action::Wait { ms: 10 },
            Action::RunScript {
                script: "echo".into(),
                args: vec!["done".into()],
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.wait_calls, vec![10]);
        assert_eq!(
            s.script_calls,
            vec![("echo".to_string(), vec!["done".to_string()])]
        );
        assert!(s
            .statuses
            .iter()
            .any(|status| status.contains("Waiting 10ms")));
        assert!(s.errors.is_empty());
    }

    #[test]
    fn speak_action_emits_success_and_chain_continues() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::Speak {
                text: "task complete".into(),
            },
            Action::OpenUrl {
                url: "https://example.com".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert!(s.errors.is_empty());
        assert_eq!(s.speak_calls, vec!["task complete".to_string()]);
        assert!(s
            .statuses
            .iter()
            .any(|status| status.contains("Spoke: task complete")));
        assert_eq!(s.url_calls, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn rejects_empty_speak_payload() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![Action::Speak { text: "   ".into() }]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert!(s.speak_calls.is_empty());
        assert_eq!(s.errors.len(), 1);
        assert!(s.errors[0].contains("Speak text cannot be empty"));
    }

    #[test]
    fn cancelled_run_stops_before_next_action() {
        let runtime = MockRuntime::with_cancelled();
        let node = node_with_actions(vec![
            Action::OpenUrl {
                url: "https://example.com".into(),
            },
            Action::Speak {
                text: "never runs".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert!(s.url_calls.is_empty());
        assert!(s.speak_calls.is_empty());
        assert_eq!(s.statuses, vec![ACTION_CANCELLED_MSG.to_string()]);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn sub_prompt_captures_follow_up_and_templates_next_action() {
        let runtime = MockRuntime::with_follow_up_answers(vec!["docs"]);
        let node = node_with_actions(vec![
            Action::SubPrompt {
                prompt: "Which page should I open?".into(),
            },
            Action::OpenUrl {
                url: "https://example.com/{{follow_up}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(
            s.follow_up_prompts,
            vec!["Which page should I open?".to_string()]
        );
        assert_eq!(s.speak_calls, vec!["Which page should I open?".to_string()]);
        assert_eq!(s.url_calls, vec!["https://example.com/docs".to_string()]);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn numbered_variables_map_to_follow_up_order() {
        let runtime = MockRuntime::with_follow_up_answers(vec!["alpha", "beta"]);
        let node = node_with_actions(vec![
            Action::SubPrompt {
                prompt: "First input?".into(),
            },
            Action::SubPrompt {
                prompt: "Second input?".into(),
            },
            Action::Speak {
                text: "A=Variable 1 B=Variable 2".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(
            s.follow_up_prompts,
            vec!["First input?".to_string(), "Second input?".to_string()]
        );
        assert!(s
            .speak_calls
            .iter()
            .any(|text| text == "A=alpha B=beta"));
        assert!(s.errors.is_empty());
    }

    #[test]
    fn numbered_variable_out_of_range_stays_literal() {
        let runtime = MockRuntime::with_follow_up_answers(vec!["alpha"]);
        let node = node_with_actions(vec![
            Action::SubPrompt {
                prompt: "Only input?".into(),
            },
            Action::Speak {
                text: "Keep Variable 2 here".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert!(s
            .speak_calls
            .iter()
            .any(|text| text == "Keep Variable 2 here"));
        assert!(s.errors.is_empty());
    }

    #[test]
    fn sub_prompt_emits_plain_follow_up_status_text() {
        let runtime = MockRuntime::with_follow_up_answers(vec!["docs"]);
        let node = node_with_actions(vec![Action::SubPrompt {
            prompt: "Which page should I open?".into(),
        }]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert!(s.statuses.iter().any(|status| status == "follow up"));
        assert!(s.statuses.iter().any(|status| status == "docs"));
        assert!(s.errors.is_empty());
    }

    #[test]
    fn remainder_template_substitutes_in_open_app_name() {
        let ctx = ToolCallContext::with_remainder("notepad");
        let exec_ctx = ExecutionContext::from_tool_context(Some(&ctx));
        let resolved = resolve_action_templates(
            &Action::OpenApp {
                name: "{{remainder}}".into(),
                path: "".into(),
                placement: None,
            },
            &exec_ctx,
            Some(&ctx),
        );
        assert_eq!(
            resolved,
            Action::OpenApp {
                name: "notepad".into(),
                path: "".into(),
                placement: None,
            }
        );
    }

    #[test]
    fn open_app_remainder_resolves_from_index() {
        let runtime = MockRuntime::default();
        let index = vec![AppEntry {
            display_name: "Notepad".into(),
            exe_path: "notepad.exe".into(),
            icon_data_url: None,
        }];
        let node = node_with_actions(vec![Action::OpenApp {
            name: "{{remainder}}".into(),
            path: "".into(),
            placement: None,
        }]);
        let ctx = ToolCallContext::with_remainder("notepad");
        execute_actions(
            &node.actions,
            &runtime,
            Some(&index),
            Some(&ctx),
            None,
            None,
            None,
            &mut ExecutionContext::from_tool_context(Some(&ctx)),
            &mut RunState::default(),
            None,
        );
        let s = runtime.snapshot();
        assert_eq!(s.app_calls, vec!["notepad.exe".to_string()]);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn tool_arg_templates_substitute_param_placeholders() {
        let ctx = ToolCallContext::from_args(&[(
            "url".to_string(),
            "https://example.com".to_string(),
        )]
        .into_iter()
        .collect());
        let exec_ctx = ExecutionContext::from_tool_context(Some(&ctx));
        let resolved = resolve_action_templates(
            &Action::OpenUrl {
                url: "{{url}}".into(),
            },
            &exec_ctx,
            Some(&ctx),
        );
        assert_eq!(
            resolved,
            Action::OpenUrl {
                url: "https://example.com".into()
            }
        );
    }

    #[test]
    fn sub_prompt_timeout_stops_remaining_actions() {
        let runtime = MockRuntime::with_follow_up_failure("Need input");
        let node = node_with_actions(vec![
            Action::SubPrompt {
                prompt: "Need input".into(),
            },
            Action::OpenUrl {
                url: "https://example.com/never".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.follow_up_prompts, vec!["Need input".to_string()]);
        assert!(s.url_calls.is_empty());
        assert_eq!(s.errors, vec!["Follow-up timed out".to_string()]);
    }

    #[test]
    fn speak_last_result_chains_into_open_url() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::Speak {
                text: "https://example.com/docs".into(),
            },
            Action::OpenUrl {
                url: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.speak_calls, vec!["https://example.com/docs".to_string()]);
        assert_eq!(s.url_calls, vec!["https://example.com/docs".to_string()]);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn step_template_resolves_prior_output() {
        let mut exec_ctx = ExecutionContext::default();
        exec_ctx.record_step_output(Value::Text("alpha".into()));
        exec_ctx.record_step_output(Value::Text("beta".into()));
        let resolved = resolve_action_templates(
            &Action::Speak {
                text: "first={{step_1}} second={{step_2}}".into(),
            },
            &exec_ctx,
            None,
        );
        assert_eq!(
            resolved,
            Action::Speak {
                text: "first=alpha second=beta".into(),
            }
        );
    }

    #[test]
    fn implicit_passthrough_fills_empty_open_url_from_last_result() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::Speak {
                text: "https://example.com/passthrough".into(),
            },
            Action::OpenUrl { url: "   ".into() },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(
            s.url_calls,
            vec!["https://example.com/passthrough".to_string()]
        );
        assert!(s.errors.is_empty());
    }

    #[test]
    fn test_command_returns_per_step_outputs() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::Speak {
                text: "https://example.com".into(),
            },
            Action::OpenUrl {
                url: "{{last_result}}".into(),
            },
        ]);
        let result = test_command_actions(&node, &runtime, None, None, None);
        assert_eq!(result.steps.len(), 2);
        assert_eq!(result.steps[0].action_kind, "speak");
        assert_eq!(result.steps[0].output_preview, "https://example.com");
        assert_eq!(result.steps[1].action_kind, "open_url");
        assert_eq!(
            result.steps[1].output_preview,
            "Opening https://example.com..."
        );
    }

    fn spawn_test_http_server(body: &str) -> String {
        use std::io::Write;
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        listener
            .set_nonblocking(true)
            .expect("nonblocking test server");
        let addr = listener.local_addr().expect("test server addr");
        let body = body.to_string();
        thread::spawn(move || {
            for _ in 0..200 {
                if let Ok((mut stream, _)) = listener.accept() {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
            panic!("test HTTP server did not receive a connection");
        });
        format!("http://{addr}/")
    }

    #[test]
    fn read_file_outputs_text_and_chains_to_speak() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("pipeline.txt");
        std::fs::write(&path, "pipeline data").expect("write test file");
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::ReadFile {
                path: path.to_string_lossy().into_owned(),
            },
            Action::Speak {
                text: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.speak_calls, vec!["pipeline data".to_string()]);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn http_get_returns_response_body() {
        let url = spawn_test_http_server("api payload");
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::HttpGet { url },
            Action::Speak {
                text: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.speak_calls, vec!["api payload".to_string()]);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn get_clipboard_returns_text() {
        let mut clipboard = arboard::Clipboard::new().expect("clipboard");
        clipboard
            .set_text("clipboard payload")
            .expect("set clipboard text");
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::GetClipboard {},
            Action::Speak {
                text: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.speak_calls, vec!["clipboard payload".to_string()]);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn text_match_extracts_capture_group_and_chains() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::Speak {
                text: "order-12345 shipped".into(),
            },
            Action::TextMatch {
                pattern: r"order-(\d+)".into(),
                text: "{{last_result}}".into(),
            },
            Action::Speak {
                text: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.speak_calls, vec!["order-12345 shipped".to_string(), "12345".to_string()]);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn text_split_and_combine_pipeline() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::Speak {
                text: "alpha,beta,gamma".into(),
            },
            Action::TextSplit {
                delimiter: ",".into(),
                text: "{{last_result}}".into(),
            },
            Action::TextCombine {
                separator: " | ".into(),
            },
            Action::Speak {
                text: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(
            s.speak_calls,
            vec![
                "alpha,beta,gamma".to_string(),
                "alpha | beta | gamma".to_string(),
            ]
        );
        assert!(s.errors.is_empty());
    }

    #[test]
    fn list_folder_returns_sorted_entry_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("b.txt"), "b").expect("write b");
        std::fs::write(dir.path().join("a.txt"), "a").expect("write a");
        std::fs::create_dir(dir.path().join("subdir")).expect("mkdir");
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::ListFolder {
                path: dir.path().to_string_lossy().into_owned(),
            },
            Action::TextCombine {
                separator: " | ".into(),
            },
            Action::Speak {
                text: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(
            s.speak_calls,
            vec!["a.txt | b.txt | subdir".to_string()]
        );
        assert!(s.errors.is_empty());
    }

    #[test]
    fn write_file_persists_text_for_read_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.txt");
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::Speak {
                text: "saved data".into(),
            },
            Action::WriteFile {
                path: path.to_string_lossy().into_owned(),
                content: "{{last_result}}".into(),
            },
            Action::ReadFile {
                path: path.to_string_lossy().into_owned(),
            },
            Action::Speak {
                text: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(
            s.speak_calls,
            vec!["saved data".to_string(), "saved data".to_string()]
        );
        assert!(s.errors.is_empty());
    }

    #[test]
    fn get_file_metadata_includes_size_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("meta.txt");
        std::fs::write(&path, "12345").expect("write");
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::GetFileMetadata {
                path: path.to_string_lossy().into_owned(),
            },
            Action::Speak {
                text: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.speak_calls.len(), 1);
        assert!(s.speak_calls[0].contains("size_bytes="));
        assert!(s.speak_calls[0].contains("meta.txt"));
        assert!(s.errors.is_empty());
    }

    #[test]
    fn file_metadata_map_reports_file_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("meta.txt");
        std::fs::write(&path, "12345").expect("write");
        let map = file_metadata_map(path.to_str().expect("utf8 path")).expect("metadata");
        assert_eq!(map.get("size_bytes"), Some(&Value::Number(5.0)));
        assert_eq!(
            map.get("name"),
            Some(&Value::Text("meta.txt".to_string()))
        );
    }

    #[test]
    fn device_info_map_includes_memory_fields() {
        let map = device_info_map();
        assert!(map.contains_key("total_memory_bytes"));
        assert!(map.contains_key("cpu_usage_pct"));
    }

    #[test]
    fn device_info_dict_serializes_for_chaining() {
        let map = device_info_map();
        let text = Value::Dict(map).coerce_to_text();
        assert!(
            text.contains("total_memory_bytes="),
            "device info preview: {text}"
        );
    }

    #[test]
    fn device_info_chains_to_speak() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::DeviceInfo {},
            Action::Speak {
                text: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.speak_calls.len(), 1);
        assert!(s.speak_calls[0].contains("total_memory_bytes="));
        assert!(s.errors.is_empty());
    }

    #[test]
    fn screenshot_writes_png_to_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("cap.png");
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::Screenshot {
                path: Some(out.to_string_lossy().into_owned()),
            },
            Action::Speak {
                text: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.speak_calls.len(), 1);
        let saved = std::path::Path::new(&s.speak_calls[0]);
        assert!(saved.is_file(), "expected screenshot file at {}", saved.display());
        assert!(std::fs::metadata(saved).expect("stat").len() > 0);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn set_clipboard_writes_text_from_prior_step() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::Speak {
                text: "copied payload".into(),
            },
            Action::SetClipboard {
                text: "{{last_result}}".into(),
            },
            Action::GetClipboard {},
            Action::Speak {
                text: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.speak_calls, vec!["copied payload".to_string(), "copied payload".to_string()]);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn if_else_runs_then_branch_when_text_contains() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::Speak {
                text: "hello world".into(),
            },
            Action::IfElse {
                condition: IfConditionKind::TextContains,
                text: "{{last_result}}".into(),
                pattern: "world".into(),
                then_actions: vec![Action::Speak {
                    text: "matched".into(),
                }],
                else_actions: vec![Action::Speak {
                    text: "not matched".into(),
                }],
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(
            s.speak_calls,
            vec!["hello world".to_string(), "matched".to_string()]
        );
        assert!(s.errors.is_empty());
    }

    #[test]
    fn if_else_runs_else_branch_when_text_is_empty() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::TextTrim {
                text: "   ".into(),
            },
            Action::IfElse {
                condition: IfConditionKind::TextIsEmpty,
                text: "{{last_result}}".into(),
                pattern: String::new(),
                then_actions: vec![Action::Speak {
                    text: "empty".into(),
                }],
                else_actions: vec![Action::Speak {
                    text: "not empty".into(),
                }],
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(s.speak_calls, vec!["empty".to_string()]);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn text_trim_strips_whitespace_and_chains_to_speak() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![
            Action::Speak {
                text: "  hello world  ".into(),
            },
            Action::TextTrim {
                text: "{{last_result}}".into(),
            },
            Action::Speak {
                text: "{{last_result}}".into(),
            },
        ]);

        execute_command_with_context(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(
            s.speak_calls,
            vec!["  hello world  ".to_string(), "hello world".to_string()]
        );
        assert!(s.errors.is_empty());
    }

    #[test]
    fn show_notification_uses_runtime_and_emits_nothing_value() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![Action::ShowNotification {
            title: "Done".into(),
            body: "Pipeline finished".into(),
        }]);

        let result = test_command_actions(&node, &runtime, None, None, None);
        let s = runtime.snapshot();
        assert_eq!(
            s.notification_calls,
            vec![("Done".to_string(), "Pipeline finished".to_string())]
        );
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].action_kind, "show_notification");
        assert_eq!(result.steps[0].output_preview, "");
        assert!(s.errors.is_empty());
    }
}

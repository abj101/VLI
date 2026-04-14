use crate::{
    audio::tts,
    db::{Action, CommandNode},
};
use log::{debug, warn};
use reqwest::blocking::Client;
use serde::Serialize;
use serde_json::Value;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;

pub const ACTION_STATUS_EVENT: &str = "action-status";
pub const ACTION_ERROR_EVENT: &str = "action-error";
pub const TRANSCRIPT_UPDATE_EVENT: &str = "transcript-update";
const ACTION_CANCELLED_MSG: &str = "Action run cancelled";
const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_MODEL: &str = "claude-haiku-4-5";

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
    fn run_ai_mode_prompt(&self, prompt: &str) -> Result<Option<String>, String>;
    fn emit_transcript_update(&self, text: &str);
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
        let status = Command::new("cmd")
            .arg("/C")
            .arg("start")
            .arg("")
            .arg(path)
            .status()
            .map_err(|e| format!("failed to launch app `{path}`: {e}"))?;
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
        let status = Command::new(script)
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
        let status = Command::new("powershell")
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

    fn run_ai_mode_prompt(&self, prompt: &str) -> Result<Option<String>, String> {
        let api_key = match std::env::var("ANTHROPIC_API_KEY") {
            Ok(raw) if !raw.trim().is_empty() => raw.trim().to_string(),
            _ => {
                warn!("ai_mode enabled but ANTHROPIC_API_KEY is missing; skipping");
                return Ok(None);
            }
        };
        let payload = serde_json::json!({
            "model": ANTHROPIC_MODEL,
            "max_tokens": 256,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ]
        });
        let response = Client::new()
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .map_err(|e| format!("ai preview request failed: {e}"))?;
        let response = response
            .error_for_status()
            .map_err(|e| format!("ai preview request failed: {e}"))?;
        let value: Value = response
            .json()
            .map_err(|e| format!("ai preview response parse failed: {e}"))?;
        let text = value
            .get("content")
            .and_then(|items| items.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("text"))
            .and_then(|text| text.as_str())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        Ok(text)
    }

    fn emit_transcript_update(&self, text: &str) {
        let _ = self.app.emit(
            TRANSCRIPT_UPDATE_EVENT,
            serde_json::json!({ "text": text, "is_final": true }),
        );
    }
}

pub fn execute_command(node: &CommandNode, runtime: &impl ActionRuntime) {
    debug!(
        "executor: execute_command node_id={} name={:?} actions={}",
        node.id,
        node.name,
        node.actions.len()
    );
    execute_actions(&node.actions, runtime);
    run_ai_mode_preview(node, runtime);
    debug!("executor: execute_command finished node_id={}", node.id);
}

fn run_ai_mode_preview(node: &CommandNode, runtime: &impl ActionRuntime) {
    if !node.ai_mode || runtime.is_cancelled() {
        return;
    }
    let Some(prompt) = node
        .sub_prompt
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        runtime.emit_status("AI preview skipped: missing sub_prompt");
        return;
    };
    match runtime.run_ai_mode_prompt(prompt) {
        Ok(Some(response_text)) => runtime.emit_transcript_update(&response_text),
        Ok(None) => {}
        Err(err) => {
            runtime.emit_status(&format!("AI preview failed: {err}"));
            runtime.emit_error(&err);
        }
    }
}

fn execute_actions(actions: &[Action], runtime: &impl ActionRuntime) {
    let mut follow_up_response: Option<String> = None;
    for action in actions {
        if runtime.is_cancelled() {
            runtime.emit_status(ACTION_CANCELLED_MSG);
            return;
        }
        let resolved = resolve_action_templates(action, follow_up_response.as_deref());
        match execute_one_action(&resolved, runtime) {
            Ok(text) => runtime.emit_status(&text),
            Err(err) => {
                if err == ACTION_CANCELLED_MSG {
                    runtime.emit_status(ACTION_CANCELLED_MSG);
                    return;
                }
                runtime.emit_status(&format!("Failed: {err}"));
                runtime.emit_error(&err);
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
                    follow_up_response = Some(response);
                    runtime.emit_status("Captured follow-up input");
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

fn execute_one_action(action: &Action, runtime: &impl ActionRuntime) -> Result<String, String> {
    match action {
        Action::OpenApp { name, path } => {
            validate_open_app_path(path)?;
            runtime.open_app(path)?;
            Ok(format!("Opening {name}..."))
        }
        Action::OpenUrl { url } => {
            validate_open_url(url)?;
            runtime.open_url(url)?;
            Ok(format!("Opening {url}..."))
        }
        Action::RunScript { script, args } => {
            validate_run_script(script, args)?;
            runtime.run_script(script, args)?;
            Ok(format!("Ran script {script}"))
        }
        Action::SendKeys { keys } => {
            validate_send_keys(keys)?;
            runtime.send_keys(keys)?;
            Ok("Sent key sequence".to_string())
        }
        Action::Wait { ms } => {
            validate_wait_ms(*ms)?;
            runtime.wait_ms(*ms)?;
            Ok(format!("Waiting {ms}ms..."))
        }
        Action::Speak { text } => {
            validate_speak_text(text)?;
            runtime.speak(text)?;
            Ok(format!("Spoke: {text}"))
        }
        Action::SubPrompt { prompt } => {
            validate_sub_prompt(prompt)?;
            Ok("follow up".to_string())
        }
    }
}

fn resolve_action_templates(action: &Action, follow_up_response: Option<&str>) -> Action {
    let Some(response) = follow_up_response else {
        return action.clone();
    };
    let render = |input: &str| input.replace("{{follow_up}}", response);
    match action {
        Action::OpenApp { name, path } => Action::OpenApp {
            name: render(name),
            path: render(path),
        },
        Action::OpenUrl { url } => Action::OpenUrl { url: render(url) },
        Action::RunScript { script, args } => Action::RunScript {
            script: render(script),
            args: args.iter().map(|arg| render(arg)).collect(),
        },
        Action::SendKeys { keys } => Action::SendKeys { keys: render(keys) },
        Action::Wait { ms } => Action::Wait { ms: *ms },
        Action::Speak { text } => Action::Speak { text: render(text) },
        Action::SubPrompt { prompt } => Action::SubPrompt {
            prompt: render(prompt),
        },
    }
}

fn validate_open_app_path(path: &str) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("OpenApp path cannot be empty".to_string());
    }
    if trimmed.chars().any(is_shell_metachar) {
        return Err(format!(
            "OpenApp path contains forbidden shell metacharacters: `{trimmed}`"
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

fn is_shell_metachar(c: char) -> bool {
    matches!(
        c,
        '&' | '|' | '<' | '>' | '^' | '%' | '!' | ';' | '`' | '"' | '\'' | '\n' | '\r'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn node_with_actions(actions: Vec<Action>) -> CommandNode {
        CommandNode {
            id: 1,
            name: "test".into(),
            trigger_phrases: vec!["test".into()],
            actions,
            enabled: true,
            fuzzy_threshold_pct: 80,
            ai_mode: false,
            sub_prompt: None,
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
        cancelled: bool,
        ai_prompts: Vec<String>,
        ai_result: Option<String>,
        ai_error: Option<String>,
        transcript_updates: Vec<String>,
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

        fn with_ai_result(result: &str) -> Self {
            let mut s = MockState::default();
            s.ai_result = Some(result.to_string());
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
                cancelled: self.cancelled,
                ai_prompts: self.ai_prompts.clone(),
                ai_result: self.ai_result.clone(),
                ai_error: self.ai_error.clone(),
                transcript_updates: self.transcript_updates.clone(),
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

        fn run_ai_mode_prompt(&self, prompt: &str) -> Result<Option<String>, String> {
            let mut s = self.state.lock().unwrap();
            s.ai_prompts.push(prompt.to_string());
            if let Some(err) = s.ai_error.clone() {
                return Err(err);
            }
            Ok(s.ai_result.clone())
        }

        fn emit_transcript_update(&self, text: &str) {
            self.state
                .lock()
                .unwrap()
                .transcript_updates
                .push(text.to_string());
        }
    }

    #[test]
    fn rejects_shell_metacharacters_in_open_app_path() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![Action::OpenApp {
            name: "calc".into(),
            path: "calc.exe & whoami".into(),
        }]);

        execute_command(&node, &runtime);
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

        execute_command(&node, &runtime);
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
            },
            Action::OpenUrl {
                url: "https://github.com".into(),
            },
        ]);

        execute_command(&node, &runtime);
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
            },
            Action::OpenUrl {
                url: "https://github.com".into(),
            },
        ]);

        execute_command(&node, &runtime);
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

        execute_command(&node, &runtime);
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

        execute_command(&node, &runtime);
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

        execute_command(&node, &runtime);
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

        execute_command(&node, &runtime);
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

        execute_command(&node, &runtime);
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

        execute_command(&node, &runtime);
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

        execute_command(&node, &runtime);
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

        execute_command(&node, &runtime);
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
    fn sub_prompt_emits_plain_follow_up_status_text() {
        let runtime = MockRuntime::with_follow_up_answers(vec!["docs"]);
        let node = node_with_actions(vec![Action::SubPrompt {
            prompt: "Which page should I open?".into(),
        }]);

        execute_command(&node, &runtime);
        let s = runtime.snapshot();
        assert!(s.statuses.iter().any(|status| status == "follow up"));
        assert!(s.errors.is_empty());
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

        execute_command(&node, &runtime);
        let s = runtime.snapshot();
        assert_eq!(s.follow_up_prompts, vec!["Need input".to_string()]);
        assert!(s.url_calls.is_empty());
        assert_eq!(s.errors, vec!["Follow-up timed out".to_string()]);
    }

    #[test]
    fn ai_mode_disabled_does_not_call_preview_api() {
        let runtime = MockRuntime::with_ai_result("preview text");
        let mut node = node_with_actions(vec![Action::Speak {
            text: "done".into(),
        }]);
        node.ai_mode = false;
        node.sub_prompt = Some("Summarize this".into());

        execute_command(&node, &runtime);
        let s = runtime.snapshot();
        assert!(s.ai_prompts.is_empty());
        assert!(s.transcript_updates.is_empty());
    }

    #[test]
    fn ai_mode_enabled_emits_transcript_update_from_api_response() {
        let runtime = MockRuntime::with_ai_result("ai reply");
        let mut node = node_with_actions(vec![]);
        node.ai_mode = true;
        node.sub_prompt = Some("Summarize this".into());

        execute_command(&node, &runtime);
        let s = runtime.snapshot();
        assert_eq!(s.ai_prompts, vec!["Summarize this".to_string()]);
        assert_eq!(s.transcript_updates, vec!["ai reply".to_string()]);
    }

    #[test]
    fn ai_mode_enabled_skips_emit_when_api_returns_none() {
        let runtime = MockRuntime::default();
        let mut node = node_with_actions(vec![]);
        node.ai_mode = true;
        node.sub_prompt = Some("Summarize this".into());

        execute_command(&node, &runtime);
        let s = runtime.snapshot();
        assert_eq!(s.ai_prompts, vec!["Summarize this".to_string()]);
        assert!(s.transcript_updates.is_empty());
    }
}

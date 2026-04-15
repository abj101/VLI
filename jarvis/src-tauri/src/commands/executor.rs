use crate::{
    apps::AppEntry,
    audio::tts,
    db::{Action, CommandNode},
};
use log::debug;
use serde::Serialize;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;

pub const ACTION_STATUS_EVENT: &str = "action-status";
pub const ACTION_ERROR_EVENT: &str = "action-error";
const ACTION_CANCELLED_MSG: &str = "Action run cancelled";

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
}

pub fn execute_command(
    node: &CommandNode,
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
) {
    debug!(
        "executor: execute_command node_id={} name={:?} actions={}",
        node.id,
        node.name,
        node.actions.len()
    );
    execute_actions(&node.actions, runtime, app_index);
    debug!("executor: execute_command finished node_id={}", node.id);
}

fn execute_actions(
    actions: &[Action],
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
) {
    let mut follow_up_response: Option<String> = None;
    for action in actions {
        if runtime.is_cancelled() {
            runtime.emit_status(ACTION_CANCELLED_MSG);
            return;
        }
        let resolved = resolve_action_templates(action, follow_up_response.as_deref());
        match execute_one_action(&resolved, runtime, app_index) {
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

fn execute_one_action(
    action: &Action,
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
) -> Result<String, String> {
    match action {
        Action::OpenApp { name, path } => {
            let trimmed = path.trim();
            if trimmed.is_empty() {
                let n = name.trim();
                if n.is_empty() {
                    return Err("OpenApp path cannot be empty".to_string());
                }
                if let Some(entries) = app_index {
                    if let Some(hit) = crate::apps::resolve_app(n, entries) {
                        validate_open_app_path(&hit.exe_path)?;
                        runtime.open_app(&hit.exe_path)?;
                        return Ok(format!("Opening {name}..."));
                    }
                }
                validate_open_app_start_fallback(n)?;
                runtime.open_app(n)?;
                return Ok(format!("Opening {name}..."));
            }
            validate_open_app_path(trimmed)?;
            runtime.open_app(trimmed)?;
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
        }]);
        execute_command(&node, &runtime, Some(&index));
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
        }]);
        execute_command(&node, &runtime, Some(&[]));
        let s = runtime.snapshot();
        assert_eq!(s.app_calls, vec!["notepad".to_string()]);
    }

    #[test]
    fn rejects_shell_metacharacters_in_open_app_path() {
        let runtime = MockRuntime::default();
        let node = node_with_actions(vec![Action::OpenApp {
            name: "calc".into(),
            path: "calc.exe & whoami".into(),
        }]);

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
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

        execute_command(&node, &runtime, None);
        let s = runtime.snapshot();
        assert_eq!(s.follow_up_prompts, vec!["Need input".to_string()]);
        assert!(s.url_calls.is_empty());
        assert_eq!(s.errors, vec!["Follow-up timed out".to_string()]);
    }
}

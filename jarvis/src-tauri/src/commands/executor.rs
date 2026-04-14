use crate::db::{Action, CommandNode};
use serde::Serialize;
use std::process::Command;
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;

pub const ACTION_STATUS_EVENT: &str = "action-status";
pub const ACTION_ERROR_EVENT: &str = "action-error";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionStatus {
    pub text: String,
}

pub trait ActionRuntime {
    fn open_app(&self, path: &str) -> Result<(), String>;
    fn open_url(&self, url: &str) -> Result<(), String>;
    fn emit_status(&self, text: &str);
    fn emit_error(&self, message: &str);
}

pub struct TauriActionRuntime<'a> {
    app: &'a AppHandle,
}

impl<'a> TauriActionRuntime<'a> {
    pub fn new(app: &'a AppHandle) -> Self {
        Self { app }
    }
}

impl ActionRuntime for TauriActionRuntime<'_> {
    fn open_app(&self, path: &str) -> Result<(), String> {
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
        self.app
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| format!("failed to open url `{url}`: {e}"))
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

pub fn execute_command(node: &CommandNode, runtime: &impl ActionRuntime) {
    execute_actions(&node.actions, runtime);
}

fn execute_actions(actions: &[Action], runtime: &impl ActionRuntime) {
    for action in actions {
        match execute_one_action(action, runtime) {
            Ok(text) => runtime.emit_status(&text),
            Err(err) => {
                runtime.emit_status(&format!("Failed: {err}"));
                runtime.emit_error(&err);
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
            created_at: "now".into(),
        }
    }

    #[derive(Default, Debug)]
    struct MockState {
        app_calls: Vec<String>,
        url_calls: Vec<String>,
        statuses: Vec<String>,
        errors: Vec<String>,
        fail_app_paths: Vec<String>,
        fail_urls: Vec<String>,
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

        fn snapshot(&self) -> MockState {
            self.state.lock().unwrap().clone()
        }
    }

    impl Clone for MockState {
        fn clone(&self) -> Self {
            Self {
                app_calls: self.app_calls.clone(),
                url_calls: self.url_calls.clone(),
                statuses: self.statuses.clone(),
                errors: self.errors.clone(),
                fail_app_paths: self.fail_app_paths.clone(),
                fail_urls: self.fail_urls.clone(),
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

        fn emit_status(&self, text: &str) {
            self.state.lock().unwrap().statuses.push(text.to_string());
        }

        fn emit_error(&self, message: &str) {
            self.state.lock().unwrap().errors.push(message.to_string());
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
}

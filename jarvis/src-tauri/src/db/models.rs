use serde::{Deserialize, Serialize};

/// How trigger phrases match transcript text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// Full phrase match anywhere in the transcript (default).
    #[default]
    Phrase,
    /// Trigger at a word boundary; words after the trigger become `remainder`.
    Prefix,
}

/// Persisted command definition loaded from SQLite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandNode {
    pub id: i64,
    pub name: String,
    pub trigger_phrases: Vec<String>,
    pub actions: Vec<Action>,
    pub enabled: bool,
    pub fuzzy_threshold_pct: u16,
    #[serde(default)]
    pub match_mode: MatchMode,
    pub created_at: String,
}

/// Insert payload (DB assigns `id` and `created_at` unless overridden by SQL defaults).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewCommandNode {
    pub name: String,
    pub trigger_phrases: Vec<String>,
    pub actions: Vec<Action>,
    pub enabled: bool,
    pub fuzzy_threshold_pct: u16,
    #[serde(default)]
    pub match_mode: MatchMode,
}

/// JSON-schema-style slot declared on a user- or builtin-defined tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ToolParameter {
    pub name: String,
    pub param_type: String,
    pub description: Option<String>,
    pub required: bool,
    #[serde(default)]
    pub enum_values: Vec<String>,
}

/// Persisted tool definition loaded from SQLite (`tools` table).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub id: i64,
    /// Machine id used by `execute_tool` (e.g. `open_url`).
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub parameters: Vec<ToolParameter>,
    pub actions: Vec<Action>,
    pub enabled: bool,
    pub builtin: bool,
    pub created_at: String,
}

/// Insert / update payload (DB assigns `id` and `created_at` on create).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewToolDefinition {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub parameters: Vec<ToolParameter>,
    pub actions: Vec<Action>,
    pub enabled: bool,
    pub builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    OpenApp {
        name: String,
        path: String,
        #[serde(default)]
        placement: Option<String>,
    },
    OpenUrl { url: String },
    OpenTarget {
        target: String,
        #[serde(default)]
        placement: Option<String>,
    },
    PlaceWindow {
        zone: String,
        #[serde(default)]
        monitor: Option<String>,
    },
    RunScript { script: String, args: Vec<String> },
    /// Resolved from `registered_scripts` by id (LLM / tool path only).
    RunRegisteredScript {
        script_id: String,
        args: Vec<String>,
    },
    SendKeys { keys: String },
    Wait { ms: u64 },
    Speak { text: String },
    SubPrompt { prompt: String },
    /// Run another command's action chain; `input` becomes nested `shortcut_input`.
    RunCommand {
        command_id: i64,
        #[serde(default)]
        input: Option<String>,
    },
    /// Read a text file from disk; output is file contents.
    ReadFile { path: String },
    /// HTTP GET request; output is response body text.
    HttpGet { url: String },
    /// Read plain text from the system clipboard.
    GetClipboard {},
    /// Show a desktop notification; output is Nothing.
    ShowNotification { title: String, body: String },
    /// Trim leading/trailing whitespace from text.
    TextTrim { text: String },
    /// Regex match; output is first capture group or full match.
    TextMatch { pattern: String, text: String },
    /// Split text by delimiter; output is a list of text segments.
    TextSplit { delimiter: String, text: String },
    /// Join prior list (or pass through text) with a separator.
    TextCombine { separator: String },
    /// Write plain text to the system clipboard; output is Nothing.
    SetClipboard { text: String },
    /// List entries in a directory; output is a List of entry names.
    ListFolder { path: String },
    /// Write text to a file; output is Nothing.
    WriteFile { path: String, content: String },
    /// Read file metadata; output is a Dict.
    GetFileMetadata { path: String },
    /// Capture the primary display; output is a FilePath to a PNG.
    Screenshot {
        #[serde(default)]
        path: Option<String>,
    },
    /// CPU and memory snapshot; output is a Dict.
    DeviceInfo {},
    /// Branch on a text condition; runs `then_actions` or `else_actions`.
    IfElse {
        condition: IfConditionKind,
        #[serde(default)]
        text: String,
        #[serde(default)]
        pattern: String,
        then_actions: Vec<Action>,
        #[serde(default)]
        else_actions: Vec<Action>,
    },
}

/// Condition kinds for [`Action::IfElse`] (v1 logic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IfConditionKind {
    TextContains,
    RegexMatch,
    TextIsEmpty,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_variants_round_trip() {
        let actions = vec![
            Action::OpenApp {
                name: "notepad".into(),
                path: "notepad.exe".into(),
                placement: None,
            },
            Action::OpenUrl {
                url: "https://github.com".into(),
            },
            Action::RunScript {
                script: "echo".into(),
                args: vec!["hello".into()],
            },
            Action::RunRegisteredScript {
                script_id: "deploy".into(),
                args: vec!["staging".into()],
            },
            Action::SendKeys {
                keys: "ctrl+shift+n".into(),
            },
            Action::Wait { ms: 500 },
            Action::Speak {
                text: "done".into(),
            },
            Action::SubPrompt {
                prompt: "what next".into(),
            },
        ];

        let encoded = serde_json::to_string(&actions).unwrap();
        let decoded: Vec<Action> = serde_json::from_str(&encoded).unwrap();
        assert_eq!(actions, decoded);
    }

    #[test]
    fn action_open_app_round_trip() {
        let a = Action::OpenApp {
            name: "notepad".into(),
            path: "notepad.exe".into(),
            placement: None,
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: Action = serde_json::from_str(&j).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn if_else_action_round_trip() {
        let a = Action::IfElse {
            condition: IfConditionKind::TextContains,
            text: "{{last_result}}".into(),
            pattern: "go".into(),
            then_actions: vec![Action::Speak {
                text: "yes".into(),
            }],
            else_actions: vec![Action::Speak {
                text: "no".into(),
            }],
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: Action = serde_json::from_str(&j).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn action_open_url_round_trip() {
        let a = Action::OpenUrl {
            url: "https://github.com".into(),
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: Action = serde_json::from_str(&j).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn unit_action_accepts_empty_object_json() {
        for json in [r#"{"get_clipboard":{}}"#, r#"{"device_info":{}}"#] {
            let parsed: Action = serde_json::from_str(json).unwrap_or_else(|e| {
                panic!("failed to parse {json}: {e}");
            });
            match parsed {
                Action::GetClipboard {} | Action::DeviceInfo {} => {}
                other => panic!("unexpected variant for {json}: {other:?}"),
            }
        }
    }
}

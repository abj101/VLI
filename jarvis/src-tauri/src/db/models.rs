use serde::{Deserialize, Serialize};

/// Persisted command definition loaded from SQLite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandNode {
    pub id: i64,
    pub name: String,
    pub trigger_phrases: Vec<String>,
    pub actions: Vec<Action>,
    pub enabled: bool,
    pub fuzzy_threshold_pct: u16,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    OpenApp { name: String, path: String },
    OpenUrl { url: String },
    RunScript { script: String, args: Vec<String> },
    SendKeys { keys: String },
    Wait { ms: u64 },
    Speak { text: String },
    SubPrompt { prompt: String },
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
            },
            Action::OpenUrl {
                url: "https://github.com".into(),
            },
            Action::RunScript {
                script: "echo".into(),
                args: vec!["hello".into()],
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
}

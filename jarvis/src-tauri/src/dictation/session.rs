//! Dictation session lifecycle and text joining.
use crate::hud::{HudOverlayMode, HudPhase};

#[derive(Debug, Clone, Copy)]
pub struct HudSnapshot {
    pub visible: bool,
    pub phase: HudPhase,
    pub overlay_mode: HudOverlayMode,
}
impl HudSnapshot {
    pub fn blocks_dictation(self) -> bool {
        self.visible
            && self.overlay_mode == HudOverlayMode::Command
            && matches!(
                self.phase,
                HudPhase::Listening
                    | HudPhase::Matched
                    | HudPhase::Routing
                    | HudPhase::Executing
                    | HudPhase::AwaitingInput
            )
    }
}
#[derive(Debug, Default)]
pub struct DictationSessionController {
    session_id: Option<u64>,
    inserted_tail: String,
    utterance_count: usize,
    context: &'static str,
}
impl DictationSessionController {
    pub fn is_active(&self) -> bool {
        self.session_id.is_some()
    }
    pub fn start(&mut self, session_id: u64, hud: HudSnapshot) -> Result<u64, String> {
        if self.is_active() {
            return Err("dictation already active".into());
        }
        if hud.blocks_dictation() {
            return Err("cannot start dictation while voice HUD is active".into());
        }
        self.session_id = Some(session_id);
        self.inserted_tail.clear();
        self.utterance_count = 0;
        Ok(session_id)
    }
    pub fn set_context(&mut self, context: &'static str) {
        self.context = context;
    }
    pub fn context(&self) -> &'static str {
        self.context
    }

    pub fn stop(&mut self) -> u64 {
        self.session_id.take().unwrap_or(0)
    }
    pub fn accepts(&self, id: u64) -> bool {
        self.session_id == Some(id)
    }
    pub fn final_text(&self, id: u64, text: &str, is_final: bool) -> Option<String> {
        if self.accepts(id) && is_final {
            Some(join_and_capitalize(self.tail(), text))
        } else {
            None
        }
    }
    pub fn tail(&self) -> &str {
        &self.inserted_tail
    }
    pub fn remember(&mut self, text: &str) {
        self.inserted_tail.push_str(text);
        self.inserted_tail = self
            .inserted_tail
            .chars()
            .rev()
            .take(200)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        self.utterance_count += 1;
    }
}
pub fn join_and_capitalize(tail: &str, text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let capitalize = tail
        .trim_end()
        .chars()
        .last()
        .is_none_or(|c| matches!(c, '.' | '?' | '!'));
    let mut chars = text.chars();
    let first = chars.next().unwrap();
    let mut result = String::new();
    if !tail.is_empty() && !tail.ends_with(char::is_whitespace) {
        result.push(' ');
    }
    if capitalize {
        result.extend(first.to_uppercase());
    } else {
        result.push(first);
    }
    result.extend(chars);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joins() {
        assert_eq!(join_and_capitalize("", "hello"), "Hello");
        assert_eq!(join_and_capitalize("Hi.", "there"), " There");
        assert_eq!(join_and_capitalize("Hi,", "there"), " there");
    }
    #[test]
    fn consecutive_finals_keep_one_space() {
        let mut ctrl = DictationSessionController::default();
        let first = join_and_capitalize(ctrl.tail(), "first sentence.");
        ctrl.remember(&first);
        let second = join_and_capitalize(ctrl.tail(), "second sentence.");
        ctrl.remember(&second);
        assert_eq!(ctrl.tail(), "First sentence. Second sentence.");
    }

    #[test]
    fn partials_never_insert_and_final_inserts_once() {
        let mut ctrl = DictationSessionController::default();
        ctrl.start(
            7,
            HudSnapshot {
                visible: false,
                phase: HudPhase::Idle,
                overlay_mode: HudOverlayMode::Command,
            },
        )
        .unwrap();
        let mut inserted = Vec::new();
        for (text, final_update) in [
            ("hel", false),
            ("hello world", false),
            ("hello world", true),
        ] {
            if let Some(text) = ctrl.final_text(7, text, final_update) {
                inserted.push(text);
            }
        }
        assert_eq!(inserted, vec!["Hello world"]);
    }
}

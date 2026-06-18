//! In-memory mirror of text typed into the focused field during dictation.

use super::reconcile::DeltaOps;

#[derive(Debug, Clone, Default)]
pub struct ScreenModel {
    text: String,
    typed_in_session: bool,
}

impl ScreenModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_parts(text: String, typed_in_session: bool) -> Self {
        Self {
            text,
            typed_in_session,
        }
    }

    pub(crate) fn into_parts(self) -> (String, bool) {
        (self.text, self.typed_in_session)
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn typed_in_session(&self) -> bool {
        self.typed_in_session
    }

    pub fn apply_delta(&mut self, delta: &DeltaOps) {
        if delta.backspaces > 0 {
            let keep = self
                .text
                .chars()
                .count()
                .saturating_sub(delta.backspaces);
            self.text = self.text.chars().take(keep).collect();
        }
        if !delta.suffix.is_empty() {
            self.text.push_str(&delta.suffix);
            self.typed_in_session = true;
        }
    }

    pub fn append_utterance(&mut self, to_type: &str) {
        self.text.push_str(to_type);
        self.typed_in_session = true;
    }
}

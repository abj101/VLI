//! Dictation session lifecycle and text joining.
use super::format::{self, FormatState};
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
    pub(super) format_state: FormatState,
    pub(super) url_mode: Option<bool>,
    waiting_since: Option<std::time::Instant>,
    has_listened: bool,
    pub(super) pending_insert: String,
}
impl DictationSessionController {
    pub fn is_active(&self) -> bool {
        self.session_id.is_some()
    }
    pub(super) fn session_id(&self) -> u64 {
        self.session_id.unwrap_or(0)
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
        self.format_state = FormatState::default();
        self.url_mode = None;
        self.waiting_since = Some(std::time::Instant::now());
        self.has_listened = false;
        self.pending_insert.clear();
        Ok(session_id)
    }
    pub fn waiting_for_input(&self) -> bool {
        self.is_active() && self.waiting_since.is_some()
    }
    pub fn waiting_expired(&self, now: std::time::Instant) -> bool {
        !self.has_listened && self.waiting_since.is_some_and(|since| {
            now.saturating_duration_since(since) >= std::time::Duration::from_secs(10)
        })
    }
    pub fn set_input_ready(&mut self, ready: bool, now: std::time::Instant) -> bool {
        if self.waiting_for_input() == !ready {
            return false;
        }
        self.waiting_since = if ready { None } else { Some(now) };
        self.has_listened |= ready;
        if ready { self.url_mode = None; }
        true
    }
    pub(super) fn queue_insert(&mut self, text: &str) {
        self.remember(text);
        self.pending_insert.push_str(text);
    }
    pub(super) fn take_pending_insert(&mut self) -> String {
        std::mem::take(&mut self.pending_insert)
    }
    pub fn set_context(&mut self, context: &'static str) {
        self.context = context;
    }
    pub fn context(&self) -> &'static str {
        self.context
    }

    pub fn stop(&mut self) -> (u64, String) {
        let text = self.finish_utterance();
        (self.session_id.take().unwrap_or(0), text)
    }
    pub(super) fn finish_utterance(&mut self) -> String {
        let mode = self.url_mode.unwrap_or(false);
        let pending = format::flush(&mut self.format_state, mode);
        let text = join_and_capitalize(self.tail(), &pending, mode);
        self.url_mode = None;
        text
    }
    pub fn accepts(&self, id: u64) -> bool {
        self.session_id == Some(id)
    }
    pub(super) fn format_words(&mut self, words: &str, url_mode: bool) -> String {
        // Reuse already inserted escape context without holding back another word.
        let previous = self.tail().split_whitespace().last().unwrap_or_default();
        if matches!(previous.to_ascii_lowercase().as_str(), "a" | "the" | "my") {
            let prefix = previous.to_owned();
            let pending = std::mem::take(&mut self.format_state.pending);
            let input = format!("{prefix} {pending} {words}");
            let formatted = format::format_chunk(&input, url_mode, &mut self.format_state);
            formatted[prefix.len()..].trim_start_matches(' ').to_owned()
        } else {
            format::format_chunk(words, url_mode, &mut self.format_state)
        }
    }
    pub fn final_text(&self, id: u64, text: &str, is_final: bool) -> Option<String> {
        if self.accepts(id) && is_final {
            Some(join_and_capitalize(
                self.tail(),
                text,
                self.url_mode.unwrap_or(false)
                    || self.format_state.pending.eq_ignore_ascii_case("dot"),
            ))
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
pub fn join_and_capitalize(tail: &str, text: &str, url_mode: bool) -> String {
    if text.is_empty() {
        return String::new();
    }
    let last_line = tail.rsplit('\n').next().unwrap_or_default().trim();
    let after_list_marker = last_line == "-"
        || last_line.strip_suffix('.').is_some_and(|number| {
            !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit())
        });
    let capitalize = !url_mode
        && !format::is_domain(text.split_whitespace().next().unwrap_or_default())
        && (tail.ends_with('\n')
            || after_list_marker
            || tail
                .trim_end()
                .chars()
                .last()
                .is_none_or(|c| matches!(c, '.' | '?' | '!')));
    let mut chars = text.chars();
    let first = chars.next().unwrap();
    let mut result = String::new();
    let attaches = matches!(first, '.' | '/' | ':' | '@' | '-' | '_' | '\n')
        || url_mode && tail.ends_with(['.', '/', ':', '@', '-', '_']);
    if !(tail.is_empty() || tail.ends_with(char::is_whitespace) || attaches) {
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
    fn waits_for_input_and_only_times_out_before_listening() {
        let mut ctrl = controller();
        let now = std::time::Instant::now();
        assert!(ctrl.waiting_for_input());
        assert!(!ctrl.waiting_expired(now));
        assert!(ctrl.waiting_expired(now + std::time::Duration::from_secs(10)));
        assert!(ctrl.set_input_ready(true, now));
        assert!(!ctrl.waiting_for_input());
        assert!(!ctrl.set_input_ready(true, now));
        assert!(ctrl.set_input_ready(false, now));
        assert!(ctrl.waiting_for_input());
        assert!(!ctrl.waiting_expired(now + std::time::Duration::from_secs(60)));
    }

    #[test]
    fn queued_text_survives_focus_loss_in_order() {
        let mut ctrl = controller();
        ctrl.queue_insert("Hello");
        let next = ctrl.final_text(7, "world", true).unwrap();
        ctrl.queue_insert(&next);
        ctrl.set_input_ready(true, std::time::Instant::now());
        ctrl.set_input_ready(false, std::time::Instant::now());
        assert_eq!(ctrl.take_pending_insert(), "Hello world");
        assert!(ctrl.take_pending_insert().is_empty());
    }
    fn controller() -> DictationSessionController {
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
        ctrl.url_mode = Some(false);
        ctrl
    }
    #[test]
    fn streaming_formats_and_escapes_without_extra_holdback() {
        for (chunks, expected) in [
            (vec!["github dot", "com"], "github.com"),
            (vec!["eggs new", "line milk"], "Eggs\nMilk"),
            (vec!["bullet point", "eggs"], "- Eggs"),
            (
                vec!["number point start.", "new line two."],
                "1. Start.\nTwo.",
            ),
            (vec!["hi", "bullet point eggs"], "Hi\n- Eggs"),
            (vec!["a new", "line of products"], "A new line of products"),
            (vec!["my", "bullet point"], "My bullet point"),
        ] {
            let mut ctrl = controller();
            for words in chunks {
                let formatted = ctrl.format_words(words, false);
                let text = ctrl.final_text(7, &formatted, true).unwrap();
                ctrl.remember(&text);
            }
            assert_eq!(ctrl.tail(), expected);
        }
    }
    #[test]
    fn stop_flushes_and_start_resets_formatter() {
        let mut ctrl = controller();
        let formatted = ctrl.format_words("hello new", false);
        let text = ctrl.final_text(7, &formatted, true).unwrap();
        ctrl.remember(&text);
        assert_eq!(ctrl.stop(), (7, " new".into()));
        ctrl.start(
            8,
            HudSnapshot {
                visible: false,
                phase: HudPhase::Idle,
                overlay_mode: HudOverlayMode::Command,
            },
        )
        .unwrap();
        assert!(ctrl.format_state.pending.is_empty());
        assert!(!ctrl.format_state.mid_line);
        assert_eq!(ctrl.url_mode, None);
    }
    #[test]
    fn joins() {
        assert_eq!(join_and_capitalize("", "hello", false), "Hello");
        assert_eq!(join_and_capitalize("Hi.", "there", false), " There");
        assert_eq!(join_and_capitalize("Hi,", "there", false), " there");
        assert_eq!(join_and_capitalize("github", ".com", false), ".com");
        assert_eq!(join_and_capitalize("", "google.com", false), "google.com");
        assert_eq!(
            join_and_capitalize("", "weather in boston", true),
            "weather in boston"
        );
        assert_eq!(join_and_capitalize("eggs", "\nMilk", false), "\nMilk");
        assert_eq!(join_and_capitalize("eggs\n", "milk", false), "Milk");
        assert_eq!(join_and_capitalize("\n- ", "milk", false), "Milk");
        assert_eq!(
            join_and_capitalize("https://", "example.com", true),
            "example.com"
        );
    }
    #[test]
    fn consecutive_finals_keep_one_space() {
        let mut ctrl = DictationSessionController::default();
        let first = join_and_capitalize(ctrl.tail(), "first sentence.", false);
        ctrl.remember(&first);
        let second = join_and_capitalize(ctrl.tail(), "second sentence.", false);
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

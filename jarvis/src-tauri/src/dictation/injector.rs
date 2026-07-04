//! Text injection trait — mock in tests, Windows SendInput in production.

use crate::audio::transcript_event::TranscriptPartialKind;
use super::reconcile::{DeltaOps, ReconcileResult, TranscriptReconciler};

pub trait TextInjector {
    fn type_text(&mut self, text: &str) -> Result<(), String>;
    fn backspace(&mut self, count: usize) -> Result<(), String>;
}

/// Pause after large tail deletes so the focused control finishes before retype.
const BACKSPACE_SETTLE_THRESHOLD: usize = 12;
#[cfg(windows)]
const BACKSPACE_SETTLE_MS: u64 = 8;

pub fn apply_delta(delta: &DeltaOps, injector: &mut impl TextInjector) -> Result<(), String> {
    if delta.backspaces > 0 {
        injector.backspace(delta.backspaces)?;
        if !delta.suffix.is_empty() && delta.backspaces >= BACKSPACE_SETTLE_THRESHOLD {
            #[cfg(windows)]
            std::thread::sleep(std::time::Duration::from_millis(BACKSPACE_SETTLE_MS));
        }
    }
    if !delta.suffix.is_empty() {
        injector.type_text(&delta.suffix)?;
    }
    Ok(())
}

fn apply_reconcile_result(
    reconciler: &mut TranscriptReconciler,
    result: ReconcileResult,
    new_stt: &str,
    injector: &mut impl TextInjector,
) -> Result<(), String> {
    match &result {
        ReconcileResult::Noop | ReconcileResult::SyncLastStt(_) => {}
        ReconcileResult::Apply(delta) => apply_delta(delta, injector)?,
        ReconcileResult::AppendUtterance(text) => injector.type_text(text)?,
    }
    reconciler.commit_result(&result, new_stt);
    Ok(())
}

/// Apply one STT update via reconciler + injector.
pub fn apply_transcript(
    reconciler: &mut TranscriptReconciler,
    new_stt: &str,
    injector: &mut impl TextInjector,
) -> Result<(), String> {
    apply_transcript_with_kind(reconciler, new_stt, TranscriptPartialKind::Growth, injector)
}

/// Apply one STT update with explicit partial kind from the STT pipeline.
pub fn apply_transcript_with_kind(
    reconciler: &mut TranscriptReconciler,
    new_stt: &str,
    kind: TranscriptPartialKind,
    injector: &mut impl TextInjector,
) -> Result<(), String> {
    if matches!(kind, TranscriptPartialKind::Unchanged) {
        return Ok(());
    }
    let result = reconciler.apply_with_kind(new_stt, kind);
    apply_reconcile_result(reconciler, result, new_stt, injector)
}

/// Thin wrapper for callers still holding separate segment strings (tests / legacy).
#[cfg(test)]
pub fn apply_transcript_segment_with_kind(
    segment_injected: &mut String,
    last_stt: &mut String,
    session_typed: &mut String,
    typed_in_session: &mut bool,
    new_stt: &str,
    kind: TranscriptPartialKind,
    injector: &mut impl TextInjector,
) -> Result<(), String> {
    let mut reconciler = TranscriptReconciler::from_parts(
        std::mem::take(segment_injected),
        std::mem::take(last_stt),
        std::mem::take(session_typed),
        *typed_in_session,
    );
    apply_transcript_with_kind(&mut reconciler, new_stt, kind, injector)?;
    let parts = reconciler.into_parts();
    *segment_injected = parts.0;
    *last_stt = parts.1;
    *session_typed = parts.2;
    *typed_in_session = parts.3;
    Ok(())
}

pub struct SystemTextInjector;

impl TextInjector for SystemTextInjector {
    fn type_text(&mut self, text: &str) -> Result<(), String> {
        crate::input::type_text::type_unicode(text)
    }

    fn backspace(&mut self, count: usize) -> Result<(), String> {
        crate::input::type_text::send_backspaces(count)
    }
}

#[cfg(test)]
pub struct MockTextInjector {
    pub backspace_calls: Vec<usize>,
    pub type_calls: Vec<String>,
}

#[cfg(test)]
impl MockTextInjector {
    pub fn new() -> Self {
        Self {
            backspace_calls: Vec::new(),
            type_calls: Vec::new(),
        }
    }
}

#[cfg(test)]
impl TextInjector for MockTextInjector {
    fn type_text(&mut self, text: &str) -> Result<(), String> {
        self.type_calls.push(text.to_string());
        Ok(())
    }

    fn backspace(&mut self, count: usize) -> Result<(), String> {
        self.backspace_calls.push(count);
        Ok(())
    }
}

#[cfg(test)]
struct VirtualScreen {
    text: String,
}

#[cfg(test)]
impl VirtualScreen {
    fn new() -> Self {
        Self {
            text: String::new(),
        }
    }
}

#[cfg(test)]
impl TextInjector for VirtualScreen {
    fn type_text(&mut self, text: &str) -> Result<(), String> {
        self.text.push_str(text);
        Ok(())
    }

    fn backspace(&mut self, count: usize) -> Result<(), String> {
        let keep = self.text.chars().count().saturating_sub(count);
        self.text = self.text.chars().take(keep).collect();
        Ok(())
    }
}

#[cfg(test)]
fn replay_partials_classified(updates: &[&str]) -> String {
    use crate::audio::transcript_event::classify_partial;

    let mut reconciler = TranscriptReconciler::new();
    let mut screen = VirtualScreen::new();
    let mut last = String::new();

    for update in updates {
        let kind = if update.is_empty() {
            TranscriptPartialKind::SilenceReset
        } else {
            classify_partial(&last, update).unwrap_or(TranscriptPartialKind::Growth)
        };
        if update.is_empty() {
            last.clear();
        } else {
            last = update.to_string();
        }
        apply_transcript_with_kind(&mut reconciler, update, kind, &mut screen).unwrap();
    }
    screen.text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(
        reconciler: &mut TranscriptReconciler,
        new_stt: &str,
        mock: &mut MockTextInjector,
    ) {
        apply_transcript(reconciler, new_stt, mock).unwrap();
    }

    #[test]
    fn mock_injector_receives_delta_ops() {
        let mut reconciler = TranscriptReconciler::new();
        let mut mock = MockTextInjector::new();

        apply(&mut reconciler, "hello world", &mut mock);

        assert_eq!(mock.backspace_calls, Vec::<usize>::new());
        assert_eq!(mock.type_calls, vec!["hello world".to_string()]);
        assert_eq!(reconciler.segment_injected(), "hello world");
        assert_eq!(reconciler.screen_text(), "hello world");
    }

    #[test]
    fn growing_partial_sends_suffix_only() {
        let mut reconciler = TranscriptReconciler::new();
        let mut mock = MockTextInjector::new();

        apply(&mut reconciler, "hello", &mut mock);
        apply(&mut reconciler, "hello world", &mut mock);

        assert!(mock.backspace_calls.is_empty());
        assert_eq!(mock.type_calls, vec!["hello", " world"]);
        assert_eq!(reconciler.screen_text(), "hello world");
    }

    #[test]
    fn newline_maps_to_enter() {
        let mut reconciler = TranscriptReconciler::new();
        let mut mock = MockTextInjector::new();

        apply(&mut reconciler, "line\nbreak", &mut mock);

        assert_eq!(mock.type_calls, vec!["line\nbreak"]);
        assert!(mock.type_calls[0].contains('\n'));
    }

    #[test]
    fn new_utterance_appends_without_backspacing() {
        let mut reconciler = TranscriptReconciler::new();
        let mut mock = MockTextInjector::new();

        apply(&mut reconciler, "hello world", &mut mock);
        apply(&mut reconciler, "how are you", &mut mock);

        assert!(mock.backspace_calls.is_empty());
        assert_eq!(mock.type_calls, vec!["hello world", " how are you"]);
        assert_eq!(reconciler.segment_injected(), "how are you");
        assert_eq!(reconciler.screen_text(), "hello world how are you");
    }

    #[test]
    fn silence_reset_then_new_utterance_leads_with_space() {
        let mut reconciler =
            TranscriptReconciler::from_parts(String::new(), String::new(), String::new(), true);
        let mut mock = MockTextInjector::new();

        apply(&mut reconciler, "", &mut mock);
        apply(&mut reconciler, "next phrase", &mut mock);

        assert!(mock.backspace_calls.is_empty());
        assert_eq!(mock.type_calls, vec![" next phrase"]);
    }

    #[test]
    fn pause_resume_does_not_retype_existing_prefix() {
        let phrase = "Is there any way to put a";
        let mut reconciler = TranscriptReconciler::new();
        let mut mock = MockTextInjector::new();

        apply(&mut reconciler, phrase, &mut mock);
        apply(&mut reconciler, "", &mut mock);
        apply(&mut reconciler, "Is there any way to", &mut mock);
        apply(&mut reconciler, "Is there any way to put a banana", &mut mock);

        assert_eq!(
            mock.type_calls,
            vec![phrase.to_string(), " banana".to_string()]
        );
        assert_eq!(reconciler.segment_injected(), "Is there any way to put a banana");
    }

    #[test]
    fn pause_resume_rebuild_is_noop_when_already_typed() {
        let phrase = "Is there any way to put a";
        let mut reconciler = TranscriptReconciler::new();
        let mut mock = MockTextInjector::new();

        apply(&mut reconciler, phrase, &mut mock);
        apply(&mut reconciler, "", &mut mock);
        apply(&mut reconciler, phrase, &mut mock);

        assert_eq!(mock.type_calls, vec![phrase.to_string()]);
        assert_eq!(reconciler.segment_injected(), phrase);
    }

    #[test]
    fn slow_speech_whisper_revision_does_not_duplicate_phrases() {
        let expected = "How is this working? Is this working? Let's check. Let me type some stuff";
        let screen = replay_partials_classified(&[
            "How is this working",
            "How is this working? Is this working",
            "How is this working? Is this working? Let's check",
            "Is this working? Is this working? Let's check",
            "Is this working? Let's check. Let me type some stuff",
        ]);

        assert_eq!(screen, expected);
        assert!(
            !screen.contains("Is this working? Is this working? Let's check. is this working")
        );
    }

    #[test]
    fn case_insensitive_partial_growth_extends() {
        let mut reconciler = TranscriptReconciler::new();
        let mut mock = MockTextInjector::new();

        apply(&mut reconciler, "How is this working", &mut mock);
        apply(&mut reconciler, "how is this working? Is this", &mut mock);

        assert_eq!(mock.type_calls, vec!["How is this working", "? Is this"]);
        assert_eq!(reconciler.screen_text(), "How is this working? Is this");
    }

    #[test]
    fn whisper_prefix_revision_replaces_segment_without_duplicate() {
        let expected = "So what are we saying right now? Let's say some other stuff.";
        let screen = replay_partials_classified(&[
            "Hello. what are we saying? right now?",
            "Hello. what are we saying? right now? Let's say some other stuff.",
            "So what are we saying right now? Let's say some other stuff.",
        ]);
        assert_eq!(screen, expected);

        let screen_after_pause = replay_partials_classified(&[
            "Hello. what are we saying? right now?",
            "Hello. what are we saying? right now? Let's say some other stuff.",
            "",
            "So what are we saying right now? Let's say some other stuff.",
        ]);
        assert_eq!(screen_after_pause, expected);
    }

    #[test]
    fn revision_kind_backspaces_segment_before_retype() {
        let mut reconciler = TranscriptReconciler::new();
        let mut mock = MockTextInjector::new();
        let segment = "Hello. what are we saying? right now? Let's say some other stuff.";

        apply_transcript(&mut reconciler, "Hello. what are we saying? right now?", &mut mock).unwrap();
        apply_transcript(&mut reconciler, segment, &mut mock).unwrap();
        apply_transcript_with_kind(
            &mut reconciler,
            "So what are we saying right now? Let's say some other stuff.",
            TranscriptPartialKind::Revision,
            &mut mock,
        )
        .unwrap();

        assert_eq!(
            mock.backspace_calls.last().copied(),
            Some(segment.chars().count())
        );
        assert_eq!(
            reconciler.screen_text(),
            "So what are we saying right now? Let's say some other stuff."
        );
        assert!(!reconciler
            .screen_text()
            .contains("Hello. what are we saying"));
    }
}

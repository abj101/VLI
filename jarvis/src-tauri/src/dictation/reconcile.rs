//! STT transcript reconciliation — overlap, delta, revision, and segment policy.

use crate::audio::transcript_event::TranscriptPartialKind;
use super::screen::ScreenModel;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeltaOps {
    pub backspaces: usize,
    pub suffix: String,
}

/// Result of reconciling one STT partial against session state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileResult {
    /// No keyboard ops; sync `segment_injected` + `last_stt` to `new_stt`.
    Noop,
    /// After silence-reset: refresh `last_stt` only; keep `segment_injected`.
    SyncLastStt(String),
    /// Keyboard ops to apply; segment tracking updates after injection.
    Apply(DeltaOps),
    /// Append a new utterance (leading space when session already has text).
    AppendUtterance(String),
}

/// Owns segment + screen state; computes keyboard deltas for each STT update.
#[derive(Debug, Clone)]
pub struct TranscriptReconciler {
    segment_injected: String,
    last_stt: String,
    screen: ScreenModel,
}

impl Default for TranscriptReconciler {
    fn default() -> Self {
        Self::new()
    }
}

impl TranscriptReconciler {
    pub fn new() -> Self {
        Self {
            segment_injected: String::new(),
            last_stt: String::new(),
            screen: ScreenModel::new(),
        }
    }

    pub fn segment_injected(&self) -> &str {
        &self.segment_injected
    }

    pub fn last_stt(&self) -> &str {
        &self.last_stt
    }

    pub fn screen_text(&self) -> &str {
        self.screen.text()
    }

    pub fn from_parts(
        segment_injected: String,
        last_stt: String,
        session_typed: String,
        typed_in_session: bool,
    ) -> Self {
        let screen = ScreenModel::from_parts(session_typed, typed_in_session);
        Self {
            segment_injected,
            last_stt,
            screen,
        }
    }

    pub fn into_parts(self) -> (String, String, String, bool) {
        let (text, typed_in_session) = self.screen.into_parts();
        (
            self.segment_injected,
            self.last_stt,
            text,
            typed_in_session,
        )
    }

    /// Reconcile one STT update; caller applies `ReconcileResult` via injector + screen.
    pub fn apply(&mut self, new_stt: &str) -> ReconcileResult {
        self.apply_with_kind(new_stt, TranscriptPartialKind::Growth)
    }

    /// Like [`Self::apply`] but honors typed partial semantics from the STT pipeline.
    pub fn apply_with_kind(&mut self, new_stt: &str, kind: TranscriptPartialKind) -> ReconcileResult {
        if matches!(kind, TranscriptPartialKind::Unchanged) {
            return ReconcileResult::Noop;
        }
        if matches!(kind, TranscriptPartialKind::SilenceReset) || new_stt.is_empty() {
            self.last_stt.clear();
            return ReconcileResult::Noop;
        }

        if matches!(kind, TranscriptPartialKind::Revision) {
            return self.reconcile_revision(new_stt);
        }

        let session_typed = self.screen.text();

        if !continues_injected_segment(&self.segment_injected, &self.last_stt, new_stt) {
            return self.reconcile_new_utterance(new_stt, session_typed);
        }

        // Same rolling segment — incremental delta (or sync only after silence-reset catch-up).
        if self.last_stt.is_empty()
            && !self.segment_injected.is_empty()
            && starts_with_ci(&self.segment_injected, new_stt)
        {
            return ReconcileResult::SyncLastStt(new_stt.to_string());
        }

        let delta = apply_stt_delta(&self.segment_injected, new_stt);
        ReconcileResult::Apply(delta)
    }

    /// Apply a reconcile result to the screen buffer (injector runs keyboard ops separately).
    pub fn commit_result(&mut self, result: &ReconcileResult, new_stt: &str) {
        match result {
            ReconcileResult::Noop => {
                if new_stt.is_empty() {
                    return;
                }
                if !self.segment_injected.is_empty() && starts_with_ci(&self.segment_injected, new_stt) {
                    self.last_stt = new_stt.to_string();
                } else {
                    self.segment_injected = new_stt.to_string();
                    self.last_stt = new_stt.to_string();
                }
            }
            ReconcileResult::SyncLastStt(text) => {
                self.last_stt = text.clone();
            }
            ReconcileResult::Apply(delta) => {
                self.screen.apply_delta(delta);
                self.segment_injected = new_stt.to_string();
                self.last_stt = new_stt.to_string();
            }
            ReconcileResult::AppendUtterance(text) => {
                self.screen.append_utterance(text);
                self.segment_injected = new_stt.to_string();
                self.last_stt = new_stt.to_string();
            }
        }
    }

    /// Whisper revised wording in the same segment window — replace the injected span.
    fn reconcile_revision(&self, new_stt: &str) -> ReconcileResult {
        let session_typed = self.screen.text();
        if new_stt_already_on_screen(session_typed, new_stt) {
            return ReconcileResult::Noop;
        }

        if let Some(delta) = apply_overlap_merge(session_typed, new_stt) {
            if delta.backspaces > 0 || !delta.suffix.is_empty() {
                return ReconcileResult::Apply(delta);
            }
            return ReconcileResult::Noop;
        }

        if let Some(delta) =
            try_segment_revision_replace(&self.segment_injected, session_typed, new_stt)
        {
            return ReconcileResult::Apply(delta);
        }

        if self.segment_injected.is_empty() {
            return self.reconcile_new_utterance(new_stt, session_typed);
        }

        ReconcileResult::Apply(segment_replace_delta(
            &self.segment_injected,
            session_typed,
            new_stt,
        ))
    }

    fn reconcile_new_utterance(&self, new_stt: &str, session_typed: &str) -> ReconcileResult {
        if new_stt_already_on_screen(session_typed, new_stt) {
            return ReconcileResult::Noop;
        }

        if let Some(delta) = apply_overlap_merge(session_typed, new_stt) {
            if delta.backspaces > 0 || !delta.suffix.is_empty() {
                return ReconcileResult::Apply(delta);
            }
            // Overlap found but nothing left to type — sync segment tracking only.
            return ReconcileResult::Noop;
        }

        if let Some(delta) =
            try_segment_revision_replace(&self.segment_injected, session_typed, new_stt)
        {
            return ReconcileResult::Apply(delta);
        }

        let to_type = if self.screen.typed_in_session() {
            format!(" {new_stt}")
        } else {
            new_stt.to_string()
        };
        ReconcileResult::AppendUtterance(to_type)
    }
}

// --- delta computation ---

/// Never backspace more than typed screen text (dictation only edits the field tail).
fn clamp_backspaces_to_screen(requested: usize, session_typed: &str) -> usize {
    requested.min(session_typed.chars().count())
}

fn segment_replace_delta(segment_injected: &str, session_typed: &str, new_stt: &str) -> DeltaOps {
    DeltaOps {
        backspaces: clamp_backspaces_to_screen(segment_injected.chars().count(), session_typed),
        suffix: new_stt.to_string(),
    }
}

pub fn apply_stt_delta(segment_injected: &str, new_stt: &str) -> DeltaOps {
    if new_stt.is_empty() {
        return DeltaOps::default();
    }

    let injected_chars: Vec<char> = segment_injected.chars().collect();
    let new_chars: Vec<char> = new_stt.chars().collect();

    let prefix_len = common_prefix_len_ci(&injected_chars, &new_chars);
    let backspaces = injected_chars.len().saturating_sub(prefix_len);
    let suffix: String = new_chars[prefix_len..].iter().collect();

    DeltaOps { backspaces, suffix }
}

fn common_prefix_len_ci(a: &[char], b: &[char]) -> usize {
    let mut prefix_len = 0usize;
    for (x, y) in a.iter().zip(b.iter()) {
        if x.eq_ignore_ascii_case(y) {
            prefix_len += 1;
        } else {
            break;
        }
    }
    prefix_len
}

// --- duplicate-on-screen detection (suffix only) ---

fn new_stt_already_on_screen(session_typed: &str, new_stt: &str) -> bool {
    if new_stt.is_empty() {
        return true;
    }
    if session_typed.eq_ignore_ascii_case(new_stt) {
        return true;
    }
    session_typed
        .to_ascii_lowercase()
        .ends_with(&new_stt.to_ascii_lowercase())
}

// --- segment continuation ---

fn starts_with_ci(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let h: Vec<char> = haystack.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    if n.len() > h.len() {
        return false;
    }
    h.iter()
        .zip(n.iter())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn is_same_stt_segment(last_stt: &str, new_stt: &str) -> bool {
    if last_stt.is_empty() {
        return false;
    }
    starts_with_ci(new_stt, last_stt) || starts_with_ci(last_stt, new_stt)
}

fn continues_injected_segment(segment_injected: &str, last_stt: &str, new_stt: &str) -> bool {
    if is_same_stt_segment(last_stt, new_stt) {
        return true;
    }
    if segment_injected.is_empty() {
        return false;
    }
    starts_with_ci(new_stt, segment_injected) || starts_with_ci(segment_injected, new_stt)
}

// --- word overlap helpers ---

fn chars_through_n_words(s: &str, n: usize) -> String {
    let rest = chars_after_n_words(s, n);
    let through_len = s.len().saturating_sub(rest.len());
    s.get(..through_len).unwrap_or("").to_string()
}

fn fix_punctuation_gap_at_overlap(
    session_typed: &str,
    new_stt: &str,
    skip_words: usize,
    overlap_words: usize,
    remainder: &str,
) -> String {
    let through = chars_through_n_words(new_stt, skip_words + overlap_words);
    let Some(new_boundary) = through.chars().last() else {
        return remainder.to_string();
    };
    if !new_boundary.is_ascii_punctuation() {
        return remainder.to_string();
    }
    let Some(session_last) = session_typed.chars().last() else {
        return remainder.to_string();
    };
    if session_last.is_ascii_punctuation() || remainder.starts_with(new_boundary) {
        return remainder.to_string();
    }
    format!("{new_boundary}{remainder}")
}

fn build_overlap_delta(session_typed: &str, overlap_words: usize, remainder: String) -> DeltaOps {
    if remainder.is_empty() {
        return DeltaOps::default();
    }
    let overlap_chars = char_len_of_last_n_words(session_typed, overlap_words);
    if ends_with_overlap(session_typed, overlap_chars) {
        DeltaOps {
            backspaces: 0,
            suffix: remainder,
        }
    } else {
        DeltaOps {
            backspaces: overlap_chars,
            suffix: remainder,
        }
    }
}

fn word_suffix_prefix_overlap(typed: &str, new_stt: &str) -> Option<(usize, String)> {
    let typed_words = split_words(typed);
    let new_words = split_words(new_stt);
    if typed_words.is_empty() || new_words.is_empty() {
        return None;
    }

    let max_k = typed_words.len().min(new_words.len());
    for k in (1..=max_k).rev() {
        let suffix = &typed_words[typed_words.len() - k..];
        let prefix = &new_words[..k];
        if words_equal_ci(suffix, prefix) {
            let remainder = chars_after_n_words(new_stt, k);
            let remainder =
                fix_punctuation_gap_at_overlap(typed, new_stt, 0, k, &remainder);
            return Some((k, remainder));
        }
    }
    None
}

fn trailing_offset_word_overlap(session_typed: &str, new_stt: &str) -> Option<(usize, usize)> {
    let session_words = split_words(session_typed);
    let new_words = split_words(new_stt);
    if session_words.is_empty() || new_words.is_empty() {
        return None;
    }

    let mut best: Option<(usize, usize)> = None;
    for skip in 0..new_words.len() {
        let new_tail = &new_words[skip..];
        let max_k = session_words.len().min(new_tail.len());
        for k in (1..=max_k).rev() {
            let session_suffix = &session_words[session_words.len() - k..];
            let new_prefix = &new_tail[..k];
            if words_equal_ci(session_suffix, new_prefix) {
                let candidate = (k, skip);
                let better = best.map_or(true, |prev| {
                    k > prev.0 || (k == prev.0 && skip < prev.1)
                });
                if better {
                    best = Some(candidate);
                }
                break;
            }
        }
    }
    best
}

fn try_segment_revision_replace(
    segment_injected: &str,
    session_typed: &str,
    new_stt: &str,
) -> Option<DeltaOps> {
    const MIN_OVERLAP_WORDS: usize = 3;
    if segment_injected.is_empty() {
        return None;
    }
    if new_stt_already_on_screen(session_typed, new_stt) {
        return None;
    }

    let (overlap_words, _skip) = trailing_offset_word_overlap(session_typed, new_stt)?;
    if overlap_words < MIN_OVERLAP_WORDS {
        return None;
    }

    let segment_words = split_words(segment_injected);
    let tail_aligned = overlap_words >= segment_words.len().saturating_sub(1);
    let substantial = overlap_words >= MIN_OVERLAP_WORDS.max(segment_words.len() / 2);
    if !tail_aligned && !substantial {
        return None;
    }

    Some(segment_replace_delta(segment_injected, session_typed, new_stt))
}

fn apply_overlap_merge(session_typed: &str, new_stt: &str) -> Option<DeltaOps> {
    if let Some((k, remainder)) = word_suffix_prefix_overlap(session_typed, new_stt) {
        if remainder.is_empty() {
            return Some(DeltaOps::default());
        }
        return Some(build_overlap_delta(session_typed, k, remainder));
    }

    let (k, skip) = trailing_offset_word_overlap(session_typed, new_stt)?;
    if skip == 0 {
        return None;
    }

    let remainder = chars_after_n_words(new_stt, skip + k);
    let remainder = fix_punctuation_gap_at_overlap(session_typed, new_stt, skip, k, &remainder);
    if remainder.is_empty() {
        return None;
    }
    Some(build_overlap_delta(session_typed, k, remainder))
}

fn split_words(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

fn normalize_word(w: &str) -> String {
    w.trim_end_matches(|c: char| c.is_ascii_punctuation())
        .to_ascii_lowercase()
}

fn words_equal_ci(a: &[String], b: &[String]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| normalize_word(x) == normalize_word(y))
}

fn char_len_of_last_n_words(s: &str, n: usize) -> usize {
    let words: Vec<&str> = s.split_whitespace().collect();
    if n == 0 || n > words.len() {
        return 0;
    }
    let tail = words[words.len() - n..].join(" ");
    if words.len() == n {
        s.chars().count()
    } else {
        tail.chars().count() + 1
    }
}

fn chars_after_n_words(s: &str, n: usize) -> String {
    if n == 0 {
        return s.to_string();
    }
    let mut byte_idx = 0usize;
    let mut words_seen = 0usize;
    let bytes = s.as_bytes();
    while byte_idx < bytes.len() && words_seen < n {
        while byte_idx < bytes.len() && bytes[byte_idx].is_ascii_whitespace() {
            byte_idx += 1;
        }
        if byte_idx >= bytes.len() {
            break;
        }
        while byte_idx < bytes.len() && !bytes[byte_idx].is_ascii_whitespace() {
            byte_idx += 1;
        }
        words_seen += 1;
    }
    s.get(byte_idx..).unwrap_or("").to_string()
}

fn ends_with_overlap(session_typed: &str, overlap_chars: usize) -> bool {
    if overlap_chars == 0 {
        return true;
    }
    session_typed.chars().count() >= overlap_chars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growing_partial_extends_suffix() {
        let delta = apply_stt_delta("hello", "hello world");
        assert_eq!(delta.backspaces, 0);
        assert_eq!(delta.suffix, " world");
    }

    #[test]
    fn correction_shrinks_then_retypes() {
        let delta = apply_stt_delta("hello world", "hello worl");
        assert_eq!(delta.backspaces, 1);
        assert_eq!(delta.suffix, "");
    }

    #[test]
    fn empty_stt_is_noop() {
        let mut reconciler = TranscriptReconciler::new();
        let result = reconciler.apply("");
        assert_eq!(result, ReconcileResult::Noop);
        assert!(reconciler.last_stt().is_empty());
    }

    #[test]
    fn silence_reset_clears_stt_context_only() {
        let mut reconciler = TranscriptReconciler::new();
        let r = reconciler.apply("hello world");
        reconciler.commit_result(&r, "hello world");
        let result = reconciler.apply("");
        assert_eq!(result, ReconcileResult::Noop);
        assert_eq!(reconciler.segment_injected(), "hello world");
        assert!(reconciler.last_stt().is_empty());
    }

    #[test]
    fn fresh_segment_types_full_string() {
        let delta = apply_stt_delta("", "hello");
        assert_eq!(delta.backspaces, 0);
        assert_eq!(delta.suffix, "hello");
    }

    #[test]
    fn same_segment_detects_partial_growth_and_correction() {
        assert!(is_same_stt_segment("hello", "hello world"));
        assert!(is_same_stt_segment("hello world", "hello worl"));
        assert!(!is_same_stt_segment("", "hello"));
        assert!(!is_same_stt_segment("hello world", "how are you"));
    }

    #[test]
    fn continues_injected_segment_after_silence_reset() {
        let typed = "Is there any way to put a";
        assert!(continues_injected_segment(typed, "", "Is there"));
        assert!(continues_injected_segment(typed, "", typed));
        assert!(continues_injected_segment(typed, "", "Is there any way to put an"));
        assert!(!continues_injected_segment(typed, "", "hello world"));
    }

    #[test]
    fn case_insensitive_continuation() {
        assert!(is_same_stt_segment("How is this working", "how is this working? Is this"));
        assert!(continues_injected_segment(
            "How is this working",
            "How is this working",
            "how is this working? Is this"
        ));
    }

    #[test]
    fn case_insensitive_delta_extends() {
        let delta = apply_stt_delta("How is this working", "how is this working? Is this");
        assert_eq!(delta.backspaces, 0);
        assert_eq!(delta.suffix, "? Is this");
    }

    #[test]
    fn word_overlap_finds_lets_check() {
        let typed = "How is this working? Is this working? Let's check";
        let new = "Let's check. Let me";
        let (overlap_words, remainder) = word_suffix_prefix_overlap(typed, new).unwrap();
        assert_eq!(overlap_words, 2);
        assert_eq!(remainder, ". Let me");
    }

    #[test]
    fn overlap_merge_types_remainder_when_already_on_screen() {
        let session = "How is this working? Is this working? Let's check";
        let new = "Is this working? Let's check. Let me type some stuff";
        let delta = apply_overlap_merge(session, new).unwrap();
        assert_eq!(delta.backspaces, 0);
        assert_eq!(delta.suffix, ". Let me type some stuff");
    }

    #[test]
    fn whisper_revision_overlap_is_noop_when_fully_typed() {
        let session = "How is this working? Is this working? Let's check";
        let new = "Is this working? Is this working? Let's check";
        let delta = apply_overlap_merge(session, new).unwrap();
        assert_eq!(delta.backspaces, 0);
        assert!(delta.suffix.is_empty());
    }

    #[test]
    fn trailing_offset_overlap_finds_prefix_revision() {
        let session = "Hello. what are we saying? right now? Let's say some other stuff.";
        let new = "So what are we saying right now? Let's say some other stuff.";
        let (overlap, skip) = trailing_offset_word_overlap(session, new).unwrap();
        assert!(overlap >= 3);
        assert_eq!(skip, 1);
    }

    #[test]
    fn segment_revision_replace_backspaces_segment() {
        let segment = "Hello. what are we saying? right now? Let's say some other stuff.";
        let session = segment;
        let new = "So what are we saying right now? Let's say some other stuff.";
        let delta = try_segment_revision_replace(segment, session, new).unwrap();
        assert_eq!(delta.backspaces, segment.chars().count());
        assert_eq!(delta.suffix, new);
    }

    #[test]
    fn reconciler_growing_partial() {
        let mut reconciler = TranscriptReconciler::new();
        let r1 = reconciler.apply("hello");
        assert!(matches!(r1, ReconcileResult::AppendUtterance(_)));
        reconciler.commit_result(&r1, "hello");
        let r2 = reconciler.apply("hello world");
        if let ReconcileResult::Apply(delta) = r2 {
            assert_eq!(delta.suffix, " world");
        } else {
            panic!("expected Apply");
        }
    }

    #[test]
    fn clamp_backspaces_never_exceeds_screen_length() {
        assert_eq!(clamp_backspaces_to_screen(50, "hello"), 5);
        assert_eq!(clamp_backspaces_to_screen(3, "hello"), 3);
    }

    #[test]
    fn revision_kind_replaces_injected_segment() {
        let mut reconciler = TranscriptReconciler::new();
        let growth = reconciler.apply("Hello. what are we saying? right now?");
        reconciler.commit_result(&growth, "Hello. what are we saying? right now?");
        let growth2 = reconciler.apply_with_kind(
            "Hello. what are we saying? right now? Let's say some other stuff.",
            TranscriptPartialKind::Growth,
        );
        reconciler.commit_result(
            &growth2,
            "Hello. what are we saying? right now? Let's say some other stuff.",
        );

        let revision = reconciler.apply_with_kind(
            "So what are we saying right now? Let's say some other stuff.",
            TranscriptPartialKind::Revision,
        );
        if let ReconcileResult::Apply(delta) = revision {
            assert_eq!(
                delta.backspaces,
                "Hello. what are we saying? right now? Let's say some other stuff."
                    .chars()
                    .count()
            );
            assert_eq!(
                delta.suffix,
                "So what are we saying right now? Let's say some other stuff."
            );
        } else {
            panic!("expected Apply for revision");
        }
    }
}

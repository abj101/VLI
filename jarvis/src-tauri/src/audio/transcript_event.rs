//! STT partial transcript event contract.
//!
//! ## `transcript-update` partial semantics
//!
//! Each partial [`crate::audio::stt::TranscriptUpdate`] carries a [`TranscriptPartialKind`] so
//! downstream consumers (dictation reconciler, HUD) need not infer intent from empty strings alone.
//!
//! | Kind | `text` | Meaning |
//! |------|--------|---------|
//! | `growth` | non-empty, extends prior partial | Same utterance segment; rolling window grew |
//! | `revision` | non-empty, not a prefix extension | Same segment window but Whisper revised wording |
//! | `silence_reset` | empty | Silence gap or non-word decode cleared rolling STT context |
//! | `growth` (default) | any | Legacy clients omit `kind`; treat as growth |
//!
//! `unchanged` is never emitted — STT suppresses duplicate partials before emit.
//!
//! Final transcripts (`is_final: true`) use default `growth`; consumers should treat them as
//! segment-complete text regardless of kind.

use serde::{Deserialize, Serialize};

/// How a partial STT emission relates to the previous partial in the same listen session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptPartialKind {
    /// Same segment; `text` is a longer rolling window than the last partial.
    Growth,
    /// Same segment window but non-prefix change (Whisper revised earlier words).
    Revision,
    /// Rolling decode context cleared after silence or junk decode; `text` is empty.
    SilenceReset,
    /// Duplicate partial suppressed at source (not emitted on the wire).
    Unchanged,
}

impl Default for TranscriptPartialKind {
    fn default() -> Self {
        Self::Growth
    }
}

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

/// Classify partial `new` against the last emitted partial `last`.
///
/// Returns `None` when the update should be suppressed (`unchanged` duplicate).
/// Returns `Some(SilenceReset)` when `new` is empty and `last` was non-empty.
pub fn classify_partial(last: &str, new: &str) -> Option<TranscriptPartialKind> {
    if new == last {
        return None;
    }
    if new.is_empty() {
        if last.is_empty() {
            return None;
        }
        return Some(TranscriptPartialKind::SilenceReset);
    }
    if last.is_empty() {
        return Some(TranscriptPartialKind::Growth);
    }
    if starts_with_ci(new, last) || starts_with_ci(last, new) {
        return Some(TranscriptPartialKind::Growth);
    }
    Some(TranscriptPartialKind::Revision)
}

/// Peak amplitude (0..1) must exceed this before Whisper / remote STT should run.
pub(crate) const MIN_SPEECH_PEAK_FOR_DECODE: f32 = 0.02;

pub(crate) fn audio_peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |acc, &s| if s.abs() > acc { s.abs() } else { acc })
}

/// True when `samples` has enough energy to plausibly contain speech (not silence / room noise).
pub(crate) fn audio_has_speech(samples: &[f32]) -> bool {
    audio_peak(samples) > MIN_SPEECH_PEAK_FOR_DECODE
}

fn is_word_like_token(token: &str) -> bool {
    let mut letters = 0usize;
    let mut digits = 0usize;
    let mut total = 0usize;
    for ch in token.chars() {
        if ch.is_whitespace() {
            continue;
        }
        total += 1;
        if ch.is_alphabetic() {
            letters += 1;
        } else if ch.is_ascii_digit() {
            digits += 1;
        }
    }
    if total == 0 {
        return false;
    }
    letters >= 2 || (letters >= 1 && digits >= 1 && total <= 8)
}

/// Strip Whisper-style bracket/paren stage markers: `[Music]`, `(applause)`, etc.
fn strip_bracket_markers(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '[' || ch == '(' {
            let close = if ch == '[' { ']' } else { ')' };
            let mut depth = 1usize;
            for c in chars.by_ref() {
                if c == ch {
                    depth += 1;
                } else if c == close {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

const FILLER_TOKENS: &[&str] = &[
    "uh", "um", "hm", "hmm", "mmm", "oh", "ah", "er", "eh", "mm", "mhm",
];

/// Outro / silence hallucinations Whisper often emits on noise-only audio.
const HALLUCINATION_PHRASES: &[&str] = &[
    "thanks for watching",
    "thank you for watching",
    "thanks for listening",
    "thank you for listening",
    "like and subscribe",
    "see you next time",
    "see you in the next video",
    "blank audio",
    "silence",
    "applause",
    "laughter",
    "the end",
    "subtitle",
    "subtitles",
];

fn is_filler_only(lower: &str) -> bool {
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    !tokens.is_empty() && tokens.iter().all(|t| FILLER_TOKENS.contains(t))
}

fn is_known_hallucination(lower: &str) -> bool {
    if lower.is_empty() {
        return true;
    }
    if is_filler_only(lower) {
        return true;
    }
    HALLUCINATION_PHRASES
        .iter()
        .any(|phrase| lower == *phrase || lower.starts_with(&format!("{phrase} ")))
}

/// Keep only plausible word output from STT and normalize whitespace.
/// Returns `None` for silence/noise-like decode output.
pub(crate) fn normalize_transcript_candidate(raw: &str) -> Option<String> {
    let stripped = strip_bracket_markers(raw);
    let normalized = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    let has_word_like = normalized.split_whitespace().any(is_word_like_token);
    if !has_word_like {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();
    if is_known_hallucination(&lower) {
        return None;
    }
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_duplicate_is_suppressed() {
        assert_eq!(classify_partial("hello", "hello"), None);
    }

    #[test]
    fn classify_empty_both_suppressed() {
        assert_eq!(classify_partial("", ""), None);
    }

    #[test]
    fn classify_silence_reset_clears_segment() {
        assert_eq!(
            classify_partial("hello world", ""),
            Some(TranscriptPartialKind::SilenceReset)
        );
    }

    #[test]
    fn classify_fresh_text_is_growth() {
        assert_eq!(
            classify_partial("", "hello"),
            Some(TranscriptPartialKind::Growth)
        );
    }

    #[test]
    fn classify_prefix_extension_is_growth() {
        assert_eq!(
            classify_partial("hello", "hello world"),
            Some(TranscriptPartialKind::Growth)
        );
    }

    #[test]
    fn classify_prefix_shrink_is_growth() {
        assert_eq!(
            classify_partial("hello world", "hello"),
            Some(TranscriptPartialKind::Growth)
        );
    }

    #[test]
    fn classify_non_prefix_change_is_revision() {
        assert_eq!(
            classify_partial("hello world", "how are you"),
            Some(TranscriptPartialKind::Revision)
        );
        assert_eq!(
            classify_partial(
                "How is this working? Is this working",
                "Is this working? Let's check"
            ),
            Some(TranscriptPartialKind::Revision)
        );
    }

    #[test]
    fn classify_case_insensitive_growth() {
        assert_eq!(
            classify_partial("How is this", "how is this working"),
            Some(TranscriptPartialKind::Growth)
        );
    }

    #[test]
    fn audio_has_speech_requires_peak_above_threshold() {
        let silence = vec![0.005f32; 1600];
        let speech = vec![0.05f32; 1600];
        assert!(!audio_has_speech(&silence));
        assert!(audio_has_speech(&speech));
    }

    #[test]
    fn transcript_candidate_rejects_noise_only_text() {
        assert_eq!(normalize_transcript_candidate("... --- !!!"), None);
        assert_eq!(normalize_transcript_candidate("   "), None);
        assert_eq!(normalize_transcript_candidate("[BLANK_AUDIO]"), None);
        assert_eq!(normalize_transcript_candidate("[Music]"), None);
        assert_eq!(normalize_transcript_candidate("um uh"), None);
        assert_eq!(normalize_transcript_candidate("Thanks for watching"), None);
    }

    #[test]
    fn transcript_candidate_accepts_word_like_text() {
        assert_eq!(
            normalize_transcript_candidate("  open   notepad now  "),
            Some("open notepad now".into())
        );
        assert_eq!(
            normalize_transcript_candidate("thank you"),
            Some("thank you".into())
        );
    }
}

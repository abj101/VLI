//! Case-insensitive substring match of transcript against command trigger phrases.

use crate::db::{CommandNode, MatchMode};
use rapidfuzz::fuzz;
use serde::Serialize;

/// First successful match across `nodes` (in order) and each node's `trigger_phrases` (in order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatchResult {
    pub node_id: String,
    pub matched_phrase: String,
    pub span_start: usize,
    pub span_end: usize,
    /// Words after a prefix trigger (`span_end..`), trimmed; empty for phrase mode or no tail.
    pub remainder: String,
}

/// Byte offsets in `transcript` (UTF-8) suitable for slicing `&transcript[span_start..span_end]`.
pub fn match_command(transcript: &str, nodes: &[CommandNode]) -> Option<MatchResult> {
    let mut best: Option<CandidateMatch> = None;

    for (node_index, node) in nodes.iter().enumerate() {
        if !node.enabled {
            continue;
        }
        let threshold = threshold_to_ratio(node.fuzzy_threshold_pct);
        for phrase in &node.trigger_phrases {
            if phrase.trim().is_empty() {
                continue;
            }

            if let Some((span_start, span_end)) =
                find_exact_match(transcript, phrase, &node.match_mode)
            {
                let candidate = CandidateMatch {
                    phrase_len: span_end.saturating_sub(span_start),
                    score: 1.0,
                    node_index,
                    result: MatchResult {
                        node_id: node.id.to_string(),
                        matched_phrase: phrase.clone(),
                        span_start,
                        span_end,
                        remainder: String::new(),
                    },
                };
                best = pick_better(best, candidate);
            }

            let (score, span_start, span_end) =
                best_fuzzy_match(transcript, phrase, &node.match_mode);
            if score >= threshold {
                let candidate = CandidateMatch {
                    phrase_len: span_end.saturating_sub(span_start),
                    score,
                    node_index,
                    result: MatchResult {
                        node_id: node.id.to_string(),
                        matched_phrase: phrase.clone(),
                        span_start,
                        span_end,
                        remainder: String::new(),
                    },
                };
                best = pick_better(best, candidate);
            }
        }
    }

    best.map(|mut candidate| {
        if nodes
            .iter()
            .find(|n| n.id.to_string() == candidate.result.node_id)
            .is_some_and(|n| n.match_mode == MatchMode::Prefix)
        {
            candidate.result.remainder = extract_remainder(transcript, candidate.result.span_end);
        }
        candidate.result
    })
}

#[derive(Debug)]
struct CandidateMatch {
    phrase_len: usize,
    score: f64,
    node_index: usize,
    result: MatchResult,
}

fn pick_better(current: Option<CandidateMatch>, next: CandidateMatch) -> Option<CandidateMatch> {
    match current {
        None => Some(next),
        Some(prev) => {
            if next.phrase_len > prev.phrase_len {
                Some(next)
            } else if next.phrase_len < prev.phrase_len {
                Some(prev)
            } else if next.score > prev.score + 1e-9 {
                Some(next)
            } else if next.score + 1e-9 < prev.score {
                Some(prev)
            } else if next.node_index < prev.node_index {
                Some(next)
            } else {
                Some(prev)
            }
        }
    }
}

fn threshold_to_ratio(threshold_pct: u16) -> f64 {
    f64::from(threshold_pct.min(100)) / 100.0
}

fn extract_remainder(transcript: &str, span_end: usize) -> String {
    transcript.get(span_end..).unwrap_or("").trim().to_string()
}

fn find_exact_match(
    transcript: &str,
    phrase: &str,
    mode: &MatchMode,
) -> Option<(usize, usize)> {
    match mode {
        MatchMode::Phrase => find_substring_ci(transcript, phrase),
        MatchMode::Prefix => find_prefix_at_word_boundary_ci(transcript, phrase),
    }
}

fn find_prefix_at_word_boundary_ci(transcript: &str, phrase: &str) -> Option<(usize, usize)> {
    if phrase.is_empty() {
        return None;
    }
    for (start_idx, _) in transcript.char_indices() {
        if !is_word_boundary_before(transcript, start_idx) {
            continue;
        }
        if let Some(len) = prefix_match_byte_len_ci(&transcript[start_idx..], phrase) {
            let end = start_idx + len;
            if is_word_boundary_after(transcript, end) {
                return Some((start_idx, end));
            }
        }
    }
    None
}

fn is_word_boundary_before(transcript: &str, start: usize) -> bool {
    if start == 0 {
        return true;
    }
    transcript[..start]
        .chars()
        .last()
        .is_none_or(|c| c.is_whitespace())
}

fn is_word_boundary_after(transcript: &str, end: usize) -> bool {
    end >= transcript.len()
        || transcript[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace())
}

fn best_fuzzy_match(transcript: &str, phrase: &str, mode: &MatchMode) -> (f64, usize, usize) {
    if transcript.trim().is_empty() || phrase.trim().is_empty() {
        return (0.0, 0, transcript.len());
    }

    let transcript_lower = transcript.to_lowercase();
    let phrase_lower = phrase.to_lowercase();

    let mut best_score = if matches!(mode, MatchMode::Phrase) {
        fuzz::ratio(transcript_lower.chars(), phrase_lower.chars())
    } else {
        0.0
    };
    let mut best_span = (0usize, transcript.len());

    let windows = word_windows(transcript);
    if windows.is_empty() {
        return (best_score, best_span.0, best_span.1);
    }

    let phrase_word_count = phrase.split_whitespace().count().max(1);
    let min_window = phrase_word_count.saturating_sub(1).max(1);
    let max_window = (phrase_word_count + 2).min(windows.len());
    for window_size in min_window..=max_window {
        for start in 0..=windows.len().saturating_sub(window_size) {
            let end = start + window_size - 1;
            let span_start = windows[start].0;
            let span_end = windows[end].1;
            if matches!(mode, MatchMode::Prefix) {
                if !is_word_boundary_before(transcript, span_start)
                    || !is_word_boundary_after(transcript, span_end)
                {
                    continue;
                }
            }
            let chunk = &transcript[span_start..span_end];
            let score = fuzz::ratio(chunk.to_lowercase().chars(), phrase_lower.chars());
            if score > best_score {
                best_score = score;
                best_span = (span_start, span_end);
            }
        }
    }

    (best_score, best_span.0, best_span.1)
}

fn word_windows(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut in_word = false;
    let mut word_start = 0usize;

    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if in_word {
                out.push((word_start, idx));
                in_word = false;
            }
        } else if !in_word {
            in_word = true;
            word_start = idx;
        }
    }
    if in_word {
        out.push((word_start, text.len()));
    }
    out
}

fn find_substring_ci(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    for (start_idx, _) in haystack.char_indices() {
        if let Some(len) = prefix_match_byte_len_ci(&haystack[start_idx..], needle) {
            return Some((start_idx, start_idx + len));
        }
    }
    None
}

fn prefix_match_byte_len_ci(hay: &str, needle: &str) -> Option<usize> {
    let mut len = 0usize;
    let mut hay_chars = hay.chars();
    for n in needle.chars() {
        let h = hay_chars.next()?;
        if !char_eq_ci(h, n) {
            return None;
        }
        len += h.len_utf8();
    }
    Some(len)
}

fn char_eq_ci(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    let mut ia = a.to_lowercase();
    let mut ib = b.to_lowercase();
    loop {
        match (ia.next(), ib.next()) {
            (Some(x), Some(y)) if x == y => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Action;

    fn node(
        id: i64,
        name: &str,
        phrases: Vec<&str>,
        enabled: bool,
        fuzzy_threshold_pct: u16,
    ) -> CommandNode {
        node_with_mode(id, name, phrases, enabled, fuzzy_threshold_pct, MatchMode::Phrase)
    }

    fn node_with_mode(
        id: i64,
        name: &str,
        phrases: Vec<&str>,
        enabled: bool,
        fuzzy_threshold_pct: u16,
        match_mode: MatchMode,
    ) -> CommandNode {
        CommandNode {
            id,
            name: name.into(),
            trigger_phrases: phrases.into_iter().map(str::to_string).collect(),
            actions: vec![],
            enabled,
            fuzzy_threshold_pct,
            match_mode,
            created_at: "now".into(),
        }
    }

    #[test]
    fn matches_simple_phrase_and_span() {
        let nodes = vec![node(7, "n", vec!["open notepad"], true, 80)];
        let t = "please open notepad now";
        let m = match_command(t, &nodes).expect("match");
        assert_eq!(m.node_id, "7");
        assert_eq!(m.matched_phrase, "open notepad");
        assert_eq!(&t[m.span_start..m.span_end], "open notepad");
        assert_eq!(m.span_start, 7);
        assert_eq!(m.span_end, 7 + "open notepad".len());
        assert_eq!(m.remainder, "");
    }

    #[test]
    fn no_match_returns_none() {
        let nodes = vec![node(1, "n", vec!["open notepad"], true, 80)];
        assert!(match_command("nothing here", &nodes).is_none());
    }

    #[test]
    fn first_node_in_slice_wins() {
        let a = node(1, "a", vec!["alpha"], true, 80);
        let b = node(2, "b", vec!["beta"], true, 80);
        let t = "beta then alpha";
        let m = match_command(t, &[a, b]).expect("match");
        assert_eq!(m.node_id, "1");
        assert_eq!(m.matched_phrase, "alpha");
    }

    #[test]
    fn first_matching_phrase_in_list_wins() {
        let n = node(9, "multi", vec!["zzz", "hello world"], true, 80);
        let t = "say hello world today";
        let m = match_command(t, &[n]).expect("match");
        assert_eq!(m.matched_phrase, "hello world");
        assert_eq!(&t[m.span_start..m.span_end], "hello world");
    }

    #[test]
    fn case_insensitive_ascii() {
        let nodes = vec![node(3, "n", vec!["Open NOTEPAD"], true, 80)];
        let t = "please OPEN noTePad";
        let m = match_command(t, &nodes).expect("match");
        assert_eq!(m.matched_phrase, "Open NOTEPAD");
        assert_eq!(&t[m.span_start..m.span_end], "OPEN noTePad");
    }

    #[test]
    fn span_indices_utf8_bytes() {
        let nodes = vec![node(4, "n", vec!["café"], true, 80)];
        let t = "prefix café suffix";
        let m = match_command(t, &nodes).expect("match");
        assert_eq!(&t[m.span_start..m.span_end], "café");
        assert_eq!(m.span_start, "prefix ".len());
        assert_eq!(m.span_end, m.span_start + "café".len());
    }

    #[test]
    fn disabled_nodes_skipped() {
        let off = node(1, "off", vec!["visible"], false, 80);
        let on = node(2, "on", vec!["visible"], true, 80);
        let m = match_command("visible", &[off, on]).expect("match");
        assert_eq!(m.node_id, "2");
    }

    #[test]
    fn uses_open_app_node_shape_from_db() {
        let n = CommandNode {
            id: 10,
            name: "Open Notepad".into(),
            trigger_phrases: vec!["open notepad".into()],
            actions: vec![Action::OpenApp {
                name: "notepad".into(),
                path: "notepad.exe".into(),
                placement: None,
            }],
            enabled: true,
            fuzzy_threshold_pct: 80,
            match_mode: MatchMode::Phrase,
            created_at: "x".into(),
        };
        let t = "OPEN NOTEPAD";
        let m = match_command(t, &[n]).expect("match");
        assert_eq!(m.node_id, "10");
        assert_eq!(&t[m.span_start..m.span_end], "OPEN NOTEPAD");
    }

    #[test]
    fn fuzzy_matches_typo_when_score_above_threshold() {
        let n = node(11, "typo", vec!["open notepad"], true, 80);
        let t = "please open notpad now";
        let m = match_command(t, &[n]).expect("fuzzy match");
        assert_eq!(m.node_id, "11");
        assert_eq!(m.matched_phrase, "open notepad");
    }

    #[test]
    fn fuzzy_threshold_blocks_weak_match() {
        let n = node(12, "strict", vec!["open notepad"], true, 95);
        let t = "please start notepad now";
        assert!(match_command(t, &[n]).is_none());
    }

    #[test]
    fn fuzzy_tie_break_prefers_first_node_order() {
        let a = node(21, "first", vec!["open notepad"], true, 80);
        let b = node(22, "second", vec!["open notepad"], true, 80);
        let t = "please open notpad now";
        let m = match_command(t, &[a, b]).expect("match");
        assert_eq!(m.node_id, "21");
    }

    #[test]
    fn fuzzy_respects_phrase_order_for_same_node_tie() {
        let n = node(31, "order", vec!["open notepad", "open notepad"], true, 80);
        let t = "please open notpad now";
        let m = match_command(t, &[n]).expect("match");
        assert_eq!(m.matched_phrase, "open notepad");
    }

    #[test]
    fn longest_phrase_wins_over_shorter_substring() {
        let short = node(1, "short", vec!["open"], true, 80);
        let long = node(2, "long", vec!["open notepad"], true, 80);
        let t = "please open notepad now";
        let m = match_command(t, &[short, long]).expect("match");
        assert_eq!(m.node_id, "2");
        assert_eq!(m.matched_phrase, "open notepad");
    }

    #[test]
    fn prefix_mode_extracts_remainder_after_trigger() {
        let n = node_with_mode(41, "open", vec!["open"], true, 80, MatchMode::Prefix);
        let t = "open notepad";
        let m = match_command(t, std::slice::from_ref(&n)).expect("match");
        assert_eq!(m.matched_phrase, "open");
        assert_eq!(&t[m.span_start..m.span_end], "open");
        assert_eq!(m.remainder, "notepad");
    }

    #[test]
    fn prefix_mode_finds_trigger_after_leading_words() {
        let n = node_with_mode(42, "open", vec!["open"], true, 80, MatchMode::Prefix);
        let t = "please open notepad";
        let m = match_command(t, std::slice::from_ref(&n)).expect("match");
        assert_eq!(m.remainder, "notepad");
    }

    #[test]
    fn prefix_mode_rejects_word_prefix_without_boundary() {
        let n = node_with_mode(43, "open", vec!["open"], true, 80, MatchMode::Prefix);
        assert!(match_command("opening notepad", std::slice::from_ref(&n)).is_none());
    }

    #[test]
    fn prefix_mode_empty_remainder_when_trigger_only() {
        let n = node_with_mode(44, "open", vec!["open"], true, 80, MatchMode::Prefix);
        let m = match_command("open", std::slice::from_ref(&n)).expect("match");
        assert_eq!(m.remainder, "");
    }

    #[test]
    fn phrase_mode_does_not_populate_remainder() {
        let n = node(45, "open", vec!["open notepad"], true, 80);
        let m = match_command("open notepad please", std::slice::from_ref(&n)).expect("match");
        assert_eq!(m.remainder, "");
    }
}

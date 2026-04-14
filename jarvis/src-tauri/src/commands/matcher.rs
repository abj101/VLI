//! Case-insensitive substring match of transcript against command trigger phrases.

use crate::db::CommandNode;
use serde::Serialize;

/// First successful match across `nodes` (in order) and each node's `trigger_phrases` (in order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatchResult {
    pub node_id: String,
    pub matched_phrase: String,
    pub span_start: usize,
    pub span_end: usize,
}

/// Byte offsets in `transcript` (UTF-8) suitable for slicing `&transcript[span_start..span_end]`.
pub fn match_command(transcript: &str, nodes: &[CommandNode]) -> Option<MatchResult> {
    for node in nodes {
        if !node.enabled {
            continue;
        }
        for phrase in &node.trigger_phrases {
            if let Some((span_start, span_end)) = find_substring_ci(transcript, phrase) {
                return Some(MatchResult {
                    node_id: node.id.to_string(),
                    matched_phrase: phrase.clone(),
                    span_start,
                    span_end,
                });
            }
        }
    }
    None
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
    ) -> CommandNode {
        CommandNode {
            id,
            name: name.into(),
            trigger_phrases: phrases.into_iter().map(str::to_string).collect(),
            actions: vec![],
            enabled,
            created_at: "now".into(),
        }
    }

    #[test]
    fn matches_simple_phrase_and_span() {
        let nodes = vec![node(
            7,
            "n",
            vec!["open notepad"],
            true,
        )];
        let t = "please open notepad now";
        let m = match_command(t, &nodes).expect("match");
        assert_eq!(m.node_id, "7");
        assert_eq!(m.matched_phrase, "open notepad");
        assert_eq!(&t[m.span_start..m.span_end], "open notepad");
        assert_eq!(m.span_start, 7);
        assert_eq!(m.span_end, 7 + "open notepad".len());
    }

    #[test]
    fn no_match_returns_none() {
        let nodes = vec![node(1, "n", vec!["open notepad"], true)];
        assert!(match_command("nothing here", &nodes).is_none());
    }

    #[test]
    fn first_node_in_slice_wins() {
        let a = node(1, "a", vec!["alpha"], true);
        let b = node(2, "b", vec!["beta"], true);
        let t = "beta then alpha";
        let m = match_command(t, &[a, b]).expect("match");
        assert_eq!(m.node_id, "1");
        assert_eq!(m.matched_phrase, "alpha");
    }

    #[test]
    fn first_matching_phrase_in_list_wins() {
        let n = node(
            9,
            "multi",
            vec!["zzz", "hello world"],
            true,
        );
        let t = "say hello world today";
        let m = match_command(t, &[n]).expect("match");
        assert_eq!(m.matched_phrase, "hello world");
        assert_eq!(&t[m.span_start..m.span_end], "hello world");
    }

    #[test]
    fn case_insensitive_ascii() {
        let nodes = vec![node(3, "n", vec!["Open NOTEPAD"], true)];
        let t = "please OPEN noTePad";
        let m = match_command(t, &nodes).expect("match");
        assert_eq!(m.matched_phrase, "Open NOTEPAD");
        assert_eq!(&t[m.span_start..m.span_end], "OPEN noTePad");
    }

    #[test]
    fn span_indices_utf8_bytes() {
        let nodes = vec![node(4, "n", vec!["café"], true)];
        let t = "prefix café suffix";
        let m = match_command(t, &nodes).expect("match");
        assert_eq!(&t[m.span_start..m.span_end], "café");
        assert_eq!(m.span_start, "prefix ".len());
        assert_eq!(m.span_end, m.span_start + "café".len());
    }

    #[test]
    fn disabled_nodes_skipped() {
        let off = node(1, "off", vec!["visible"], false);
        let on = node(2, "on", vec!["visible"], true);
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
            }],
            enabled: true,
            created_at: "x".into(),
        };
        let t = "OPEN NOTEPAD";
        let m = match_command(t, &[n]).expect("match");
        assert_eq!(m.node_id, "10");
        assert_eq!(&t[m.span_start..m.span_end], "OPEN NOTEPAD");
    }
}

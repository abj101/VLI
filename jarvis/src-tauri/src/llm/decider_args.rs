use crate::apps::resolve_target::{is_explicit_url, normalize_spoken_url, strip_placement_suffix};
use serde_json::{Map, Value};

const OPEN_VERBS: &[&str] = &[
    "switch over to",
    "head over to",
    "navigate to",
    "take me to",
    "pull up",
    "bring up",
    "switch to",
    "go to",
    "open",
    "launch",
    "start",
    "visit",
    "run",
    "put",
];

/// Extract the free-text arguments for a d1-selected tool call.
pub fn extract_args(tool_name: &str, clause: &str) -> Option<Map<String, Value>> {
    let clause = strip_politeness(clause);
    let mut args = Map::new();

    match tool_name {
        "open_target" => {
            // Require an open verb so chit-chat never becomes an app name.
            let target = strip_required_prefix(&clause, OPEN_VERBS)?;
            let (target, placement) = strip_placement_suffix(&target);
            // "open a search for x" is a web search, not an app named "search".
            let lower = target.to_lowercase();
            if lower.starts_with("search ") || lower.starts_with("a search ") {
                return None;
            }
            insert_required(&mut args, "target", target)?;
            if let Some(placement) = placement {
                args.insert("placement".into(), Value::String(placement));
            }
        }
        "open_url" => {
            if let Some(url) = first_explicit_url(&clause) {
                insert_required(&mut args, "url", url)?;
            } else {
                let target = strip_prefix(&clause, OPEN_VERBS);
                let (target, placement) = strip_placement_suffix(&target);
                // Keep `url`: normalize_router_tool_calls requires it before it
                // can rewrite a non-URL open_url into open_target.
                insert_required(&mut args, "url", target)?;
                if let Some(placement) = placement {
                    args.insert("placement".into(), Value::String(placement));
                }
            }
        }
        "web_search" => {
            // Require a search verb: d1 alone lets chit-chat through as a query.
            let query = strip_required_prefix(
                &clause,
                &[
                    "search the web for",
                    "search the web",
                    "open a search for",
                    "do a search for",
                    "searching for",
                    "search for",
                    "look up",
                    "look for",
                    "find me",
                    "search",
                    "google",
                    "find",
                ],
            )?;
            let lower = query.to_lowercase();
            // "find the word x on this page" is find-in-page, not a web search.
            if matches!(lower.as_str(), "it" | "that" | "this")
                || (clause.to_lowercase().starts_with("find the word "))
                || lower.contains("on this page")
                || lower.contains("in this paragraph")
            {
                return None;
            }
            insert_required(&mut args, "query", query)?;
        }
        "type_text" => {
            // Require a typing verb so commands like "paste it" are never typed literally.
            let text = strip_required_prefix(&clause, &["type", "write", "dictate", "enter"])?;
            insert_required(&mut args, "text", text)?;
        }
        "say" => {
            let text = strip_required_prefix(
                &clause,
                &["read aloud", "read out", "tell me", "speak", "say"],
            )?;
            insert_required(&mut args, "text", text)?;
        }
        _ => return None,
    }

    Some(args)
}

fn insert_required(args: &mut Map<String, Value>, key: &str, value: String) -> Option<()> {
    let value = clean_text(&value);
    if value.is_empty() {
        return None;
    }
    args.insert(key.into(), Value::String(value));
    Some(())
}

fn strip_politeness(input: &str) -> String {
    let mut text = clean_text(input);
    loop {
        let stripped = strip_prefix(&text, &["can you", "could you", "would you mind", "would you", "please"]);
        if stripped == text {
            break;
        }
        text = stripped;
    }
    for suffix in [" for me", " please"] {
        if text.to_lowercase().ends_with(suffix) {
            text.truncate(text.len() - suffix.len());
        }
    }
    clean_text(&text)
}

fn strip_prefix(input: &str, prefixes: &[&str]) -> String {
    let lower = input.to_lowercase();
    for prefix in prefixes {
        if lower == *prefix {
            return String::new();
        }
        if lower
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
        {
            return clean_text(input[prefix.len()..].trim_start());
        }
    }
    clean_text(input)
}

fn strip_required_prefix(input: &str, prefixes: &[&str]) -> Option<String> {
    let lower = clean_text(input).to_lowercase();
    prefixes
        .iter()
        .any(|p| lower == *p || lower.starts_with(&format!("{p} ")))
        .then(|| strip_prefix(input, prefixes))
}

fn clean_text(input: &str) -> String {
    input
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || c == '…')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn first_explicit_url(input: &str) -> Option<String> {
    let words: Vec<_> = input.split_whitespace().collect();
    for start in 0..words.len() {
        for end in start + 1..=words.len() {
            let candidate = clean_text(&words[start..end].join(" "));
            let normalized = normalize_spoken_url(&candidate);
            if !normalized.contains(char::is_whitespace) && is_explicit_url(&normalized) {
                return Some(clean_text(&normalized));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::extract_args;
    use serde_json::json;

    #[test]
    fn extracts_eval_free_text_args() {
        for (tool, clause, expected) in [
            ("open_target", "open spotify", json!({"target":"spotify"})),
            ("open_target", "launch discord", json!({"target":"discord"})),
            (
                "open_target",
                "please can you open spotify",
                json!({"target":"spotify"}),
            ),
            (
                "open_target",
                "can you pull up notepad for me",
                json!({"target":"notepad"}),
            ),
            (
                "open_target",
                "put brave on the left",
                json!({"target":"brave","placement":"left_half"}),
            ),
            (
                "open_target",
                "open vs code on the right side",
                json!({"target":"vs code","placement":"right_half"}),
            ),
            ("open_url", "go to github.com", json!({"url":"github.com"})),
            (
                "open_url",
                "please open github dot com",
                json!({"url":"github.com"}),
            ),
            (
                "web_search",
                "search for rust traits",
                json!({"query":"rust traits"}),
            ),
            (
                "web_search",
                "google best pizza near me",
                json!({"query":"best pizza near me"}),
            ),
            (
                "web_search",
                "look up the weather in chicago",
                json!({"query":"the weather in chicago"}),
            ),
            (
                "type_text",
                "type hello world",
                json!({"text":"hello world"}),
            ),
            (
                "type_text",
                "write see you tomorrow",
                json!({"text":"see you tomorrow"}),
            ),
            ("say", "say good morning", json!({"text":"good morning"})),
        ] {
            assert_eq!(
                serde_json::Value::Object(extract_args(tool, clause).unwrap()),
                expected,
                "{tool}: {clause}"
            );
        }
    }

    #[test]
    fn rejects_empty_required_arguments_and_unknown_tools() {
        for (tool, clause) in [
            ("open_target", "please open"),
            ("open_url", "open"),
            ("web_search", "search for"),
            ("web_search", "the dog is barking outside"),
            ("web_search", "search it"),
            ("type_text", "paste it"),
            ("say", "refresh the page"),
            ("open_target", "search for spotify"),
            ("open_target", "i wonder what time it is"),
            ("web_search", "find the word invoice"),
            ("web_search", "find something on this page"),
            ("type_text", "type"),
            ("say", "say"),
            ("unknown", "anything"),
        ] {
            assert!(extract_args(tool, clause).is_none(), "{tool}: {clause}");
        }
    }

    #[test]
    fn open_url_without_url_falls_back_in_normalizer_compatible_shape() {
        assert_eq!(
            serde_json::Value::Object(
                extract_args("open_url", "can you open spotify for me").unwrap()
            ),
            json!({"url":"spotify"})
        );
        assert_eq!(
            serde_json::Value::Object(extract_args("open_url", "put brave on the left").unwrap()),
            json!({"url":"brave","placement":"left_half"})
        );

        let call = crate::llm::router::RouterToolCall {
            tool: "open_url".into(),
            args: [("url".into(), "spotify".into())].into(),
        };
        let normalized =
            crate::llm::intent_validate::normalize_router_tool_calls(vec![call], &[], &[])
                .expect("fallback open_url normalizes");
        assert_eq!(normalized[0].tool, "open_target");
        assert_eq!(
            normalized[0].args.get("target").map(String::as_str),
            Some("spotify")
        );
    }
}

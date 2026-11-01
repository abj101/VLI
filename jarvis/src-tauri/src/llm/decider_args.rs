use crate::apps::resolve_target::{is_explicit_url, normalize_spoken_url, strip_placement_suffix};
use serde_json::{Map, Value};

/// Extract the free-text arguments for a d1-selected tool call.
pub fn extract_args(tool_name: &str, clause: &str) -> Option<Map<String, Value>> {
    let clause = strip_politeness(clause);
    let mut args = Map::new();

    match tool_name {
        "open_target" => {
            let target = strip_prefix(
                &clause,
                &[
                    "pull up",
                    "bring up",
                    "switch to",
                    "go to",
                    "open",
                    "launch",
                    "start",
                    "run",
                    "put",
                ],
            );
            let (target, placement) = strip_placement_suffix(&target);
            insert_required(&mut args, "target", target)?;
            if let Some(placement) = placement {
                args.insert("placement".into(), Value::String(placement));
            }
        }
        "open_url" => {
            if let Some(url) = first_explicit_url(&clause) {
                insert_required(&mut args, "url", url)?;
            } else {
                let target = strip_prefix(
                    &clause,
                    &[
                        "pull up",
                        "bring up",
                        "switch to",
                        "go to",
                        "open",
                        "launch",
                        "start",
                        "run",
                        "put",
                    ],
                );
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
            let query = strip_prefix(
                &clause,
                &[
                    "search the web for",
                    "search the web",
                    "search for",
                    "look up",
                    "find me",
                    "search",
                    "google",
                    "find",
                ],
            );
            insert_required(&mut args, "query", query)?;
        }
        "type_text" => {
            let text = strip_prefix(&clause, &["type", "write", "dictate", "enter"]);
            insert_required(&mut args, "text", text)?;
        }
        "say" => {
            let text = strip_prefix(&clause, &["tell me", "speak", "say"]);
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
        let stripped = strip_prefix(&text, &["can you", "could you", "please"]);
        if stripped == text {
            break;
        }
        text = stripped;
    }
    let lower = text.to_lowercase();
    if lower.ends_with(" for me") {
        text.truncate(text.len() - " for me".len());
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

fn clean_text(input: &str) -> String {
    input
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || c == '…')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn first_explicit_url(input: &str) -> Option<String> {
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

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
    "tile",
    "run",
    "put",
];

/// "alt tab", "press control shift t", "hit escape" -> SendKeys "%{TAB}", "^+t", "{ESC}".
/// Needs a modifier, or "press"/"hit" before a single key, so plain words stay words.
pub fn spoken_keys(clause: &str) -> Option<String> {
    let lower = strip_politeness(clause).to_lowercase().replace('+', " ");
    let mut words: Vec<&str> = lower
        .split_whitespace()
        .filter(|w| !matches!(*w, "plus" | "key" | "keys" | "the" | "arrow"))
        .collect();
    let pressed = matches!(words.first(), Some(&("press" | "hit" | "push")));
    if pressed {
        words.remove(0);
    }
    let mut mods = String::new();
    while let Some(m) = words.first().and_then(|w| match *w {
        "control" | "ctrl" | "ctl" => Some('^'),
        "alt" | "option" => Some('%'),
        "shift" => Some('+'),
        "windows" | "win" | "super" => Some('#'),
        _ => None,
    }) {
        if !mods.contains(m) {
            mods.push(m);
        }
        words.remove(0);
    }
    if mods.is_empty() && !pressed {
        return None;
    }
    let key = match words.as_slice() {
        ["page", "up"] => "{PGUP}".to_string(),
        ["page", "down"] => "{PGDN}".to_string(),
        [word] => match *word {
            "tab" => "{TAB}".into(),
            "enter" | "return" => "{ENTER}".into(),
            "escape" | "esc" => "{ESC}".into(),
            "space" | "spacebar" => " ".into(),
            "backspace" => "{BACKSPACE}".into(),
            "delete" | "del" => "{DELETE}".into(),
            "home" => "{HOME}".into(),
            "end" => "{END}".into(),
            "up" => "{UP}".into(),
            "down" => "{DOWN}".into(),
            "left" => "{LEFT}".into(),
            "right" => "{RIGHT}".into(),
            w if w.len() == 1 && w.chars().all(|c| c.is_ascii_alphanumeric()) => w.into(),
            w if w.starts_with('f')
                && w[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) =>
            {
                format!("{{F{}}}", &w[1..])
            }
            _ => return None,
        },
        _ => return None,
    };
    Some(format!("{mods}{key}"))
}

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
        "press_keys" => {
            args.insert("keys".into(), Value::String(spoken_keys(&clause)?));
        }
        "close_app" => {
            let target = strip_required_prefix(&clause, &["shut down", "close", "quit", "exit", "kill"])?;
            let lower = target.to_lowercase();
            // "close this tab", "close the window", "close it": the focused thing, not an app.
            if lower.split_whitespace().any(|w| {
                matches!(
                    w,
                    "this" | "that" | "it" | "tab" | "tabs" | "window" | "windows" | "one" | "everything" | "all"
                )
            }) {
                return None;
            }
            let target = strip_prefix(&target, &["the"]);
            let target = target
                .strip_suffix(" app")
                .map(str::to_string)
                .unwrap_or(target);
            insert_required(&mut args, "target", target)?;
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
    let mut text = clean_text(input);
    let lower = text.to_lowercase();
    let matches = |lower: &str| {
        prefixes
            .iter()
            .any(|p| lower == *p || lower.starts_with(&format!("{p} ")))
    };
    if !matches(&lower) {
        // One-letter speech-to-text slips in long verbs: "serch", "lauch".
        let (first, rest) = lower.split_once(' ').unwrap_or((&lower, ""));
        let verb = prefixes
            .iter()
            .find(|p| p.len() >= 6 && !p.contains(' ') && one_edit_apart(first, p))?;
        text = format!("{verb} {rest}");
    }
    Some(strip_prefix(&text, prefixes))
}

fn one_edit_apart(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a == b || a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        match a.len().cmp(&b.len()) {
            std::cmp::Ordering::Less => j += 1,
            std::cmp::Ordering::Greater => i += 1,
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    edits + (a.len() - i) + (b.len() - j) <= 1
}

/// True when the clause starts with an app-opening verb. "put" and "tile" are left out:
/// "put this window on the left" is snapping, not opening.
pub(crate) fn starts_with_open_verb(clause: &str) -> bool {
    let lower = strip_politeness(clause).to_lowercase();
    OPEN_VERBS
        .iter()
        .filter(|verb| !matches!(**verb, "put" | "tile"))
        .any(|verb| lower == *verb || lower.starts_with(&format!("{verb} ")))
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
        // Longest first: "bbc dot co dot uk" must not stop at "bbc.co".
        for end in (start + 1..=words.len()).rev() {
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
    fn spoken_key_combos_parse_and_plain_words_do_not() {
        for (spoken, keys) in [
            ("alt tab", "%{TAB}"),
            ("control z", "^z"),
            ("press control shift t", "^+t"),
            ("ctrl+c", "^c"),
            ("hit escape", "{ESC}"),
            ("press enter", "{ENTER}"),
            ("alt f4", "%{F4}"),
            ("windows d", "#d"),
        ] {
            assert_eq!(super::spoken_keys(spoken).as_deref(), Some(keys), "{spoken}");
        }
        for plain in ["tab", "copy that", "type hello", "shift", "control the music"] {
            assert_eq!(super::spoken_keys(plain), None, "{plain}");
        }
    }

    #[test]
    fn close_app_needs_a_named_app() {
        assert_eq!(
            extract_args("close_app", "close spotify"),
            Some(json!({"target":"spotify"}).as_object().unwrap().clone())
        );
        assert_eq!(
            extract_args("close_app", "quit the discord app"),
            Some(json!({"target":"discord"}).as_object().unwrap().clone())
        );
        for focused in ["close this tab", "close the window", "close it", "close everything"] {
            assert_eq!(extract_args("close_app", focused), None, "{focused}");
        }
    }

    #[test]
    fn extracts_eval_free_text_args() {
        for (tool, clause, expected) in [
            ("open_target", "open spotify", json!({"target":"spotify"})),
            ("open_target", "lauch chrome", json!({"target":"chrome"})),
            ("web_search", "serch for rust traits", json!({"query":"rust traits"})),
            ("open_url", "go to docs dot google dot com", json!({"url":"docs.google.com"})),
            ("open_url", "pull up bbc dot co dot uk", json!({"url":"bbc.co.uk"})),
            ("open_url", "launch twitch dot tv", json!({"url":"twitch.tv"})),
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

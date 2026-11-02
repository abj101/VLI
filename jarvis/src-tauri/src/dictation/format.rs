//! Deterministic, local formatting for live dictation chunks.

#[derive(Debug, Default)]
pub struct FormatState {
    /// A final spoken formatting word held until the next chunk arrives.
    pub pending: String,
    /// Text has been typed on the current line, so a list item must start a new one.
    pub mid_line: bool,
}

const TLDS: &[&str] = &[
    "com", "org", "net", "edu", "gov", "io", "dev", "ai", "co", "me", "app", "info", "uk", "de",
    "fr", "in", "us", "ca", "au", "jp",
];

fn trim_domain_punctuation(s: &str) -> &str {
    s.trim_end_matches(['.', ','])
}

/// Whether a token is a domain with one of the supported top-level domains.
pub fn is_domain(word: &str) -> bool {
    let original = trim_domain_punctuation(word);
    let candidate = original.to_ascii_lowercase();
    let candidate = candidate
        .strip_prefix("https://")
        .or_else(|| candidate.strip_prefix("http://"))
        .unwrap_or(&candidate);
    let candidate = candidate.strip_prefix("www.").unwrap_or(candidate);
    let host = candidate
        .split(['/', ':', '?', '#'])
        .next()
        .unwrap_or(candidate);
    let Some((name, tld)) = host.rsplit_once('.') else {
        return false;
    };
    // "it.In" is a missing space between sentences, not a domain: Whisper writes real TLDs lowercase.
    !name.is_empty() && TLDS.contains(&tld) && original.contains(&format!(".{tld}"))
}

/// Lowercase word without the punctuation Whisper attaches ("Line," / "point.").
fn bare(word: &str) -> String {
    word.trim_end_matches(|c: char| c.is_ascii_punctuation())
        .to_ascii_lowercase()
}

fn spoken_punctuation(word: &str) -> Option<&'static str> {
    match bare(word).as_str() {
        "dot" => Some("."),
        "slash" => Some("/"),
        "colon" => Some(":"),
        "dash" | "hyphen" => Some("-"),
        "underscore" => Some("_"),
        "at" => Some("@"),
        _ => None,
    }
}

/// Outside URL mode only words that can start a hook are held, so ordinary words like "at" type
/// immediately.
fn is_holdable(word: &str, url_mode: bool) -> bool {
    let word = bare(word);
    matches!(word.as_str(), "dot" | "bullet" | "number" | "new")
        || url_mode && spoken_punctuation(&word).is_some()
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn attach(out: &mut String, value: &str) {
    if value.is_empty() {
        return;
    }
    if !out.is_empty() && !out.ends_with(' ') && !out.ends_with('\n') {
        out.push(' ');
    }
    out.push_str(value);
}

fn attach_tightly(out: &mut String, value: &str) {
    if !value.is_empty() {
        out.push_str(value);
    }
}

fn tld_candidate(word: &str) -> bool {
    let lower = bare(word);
    TLDS.contains(&lower.as_str())
}

const MAIL_PROVIDERS: &[&str] = &[
    "gmail",
    "outlook",
    "hotmail",
    "yahoo",
    "icloud",
    "proton",
    "protonmail",
    "live",
    "aol",
];

/// "at" means "@" only when an email domain follows ("at gmail", "at acme dot com", "at acme.com"),
/// so a search like "weather at noon" keeps its "at".
// ponytail: one-word lookahead; "at acme" split from "dot com" across chunks stays literal.
fn email_at(next: Option<&String>, after: Option<&String>) -> bool {
    next.is_some_and(|w| is_domain(w) || MAIL_PROVIDERS.contains(&bare(w).as_str()))
        || after.is_some_and(|w| bare(w) == "dot")
}

fn is_at(word: &str) -> bool {
    bare(word) == "at"
}

fn format_tokens(input: &str, url_mode: bool, state: &mut FormatState) -> String {
    let mut words: Vec<String> = input.split_whitespace().map(str::to_owned).collect();
    let mut out = String::new();
    let mut tight_next = false;

    if !state.pending.is_empty() {
        let pending = std::mem::take(&mut state.pending);
        let marker = bare(&pending);
        if marker == "dot" && words.first().is_some_and(|w| tld_candidate(w)) {
            let tld = words.remove(0);
            attach_tightly(
                &mut out,
                &format!(".{}", tld.trim_end_matches(['.', ',']).to_ascii_lowercase()),
            );
        } else if url_mode
            && spoken_punctuation(&marker).is_some()
            && (!is_at(&marker) || email_at(words.first(), words.get(1)))
        {
            let punct = spoken_punctuation(&marker).unwrap();
            attach_tightly(&mut out, punct);
            tight_next = true;
        } else {
            words.insert(0, pending);
        }
    }

    // A recognized domain typed as one token is lowercased; a sentence's "." or "," after it stays.
    for word in &mut words {
        if is_domain(word) {
            let end = trim_domain_punctuation(word).len();
            *word = word[..end].to_ascii_lowercase() + &word[end..];
        }
    }
    if url_mode {
        if let Some(word) = words.last_mut() {
            *word = word.trim_end_matches('.').to_owned();
        }
    }

    let mut i = 0;
    let mut capitalize_next = false;
    while i < words.len() {
        let lower = bare(&words[i]);
        let prev_lower = i.checked_sub(1).map(|j| words[j].to_ascii_lowercase());
        let escaped_hook = prev_lower
            .as_deref()
            .is_some_and(|p| matches!(p, "a" | "the" | "my"));

        // Keep a final formatting word pending so a chunk split does not lose its operand.
        if i + 1 == words.len() && is_holdable(&words[i], url_mode) {
            state.pending = words[i].clone();
            break;
        }

        let hook = if !escaped_hook
            && lower == "new"
            && words.get(i + 1).is_some_and(|w| bare(w) == "line")
        {
            Some(("\n", 2usize, true))
        } else if !escaped_hook
            && lower == "new"
            && words.get(i + 1).is_some_and(|w| bare(w) == "paragraph")
        {
            Some(("\n\n", 2usize, true))
        } else if !escaped_hook
            && lower == "bullet"
            && words.get(i + 1).is_some_and(|w| bare(w) == "point")
        {
            Some(("- ", 2usize, true))
        } else if !escaped_hook
            && lower == "number"
            && words.get(i + 1).is_some_and(|w| bare(w) == "point")
        {
            // Starts a list; "new line" continues it from what's in the field (`next_marker`).
            Some(("1. ", 2usize, true))
        } else {
            None
        };

        if let Some((replacement, consumed, cap)) = hook {
            // A list starts on its own line, breaking only when something is already on it.
            let mid_line = if out.is_empty() {
                state.mid_line
            } else {
                !out.ends_with('\n')
            };
            if (lower == "number" || lower == "bullet") && mid_line {
                out.push('\n');
            }
            out.push_str(replacement);
            capitalize_next = cap;
            i += consumed;
            continue;
        }

        if lower == "dot" && i + 1 < words.len() && tld_candidate(&words[i + 1]) {
            let tld = words[i + 1]
                .trim_end_matches(['.', ','])
                .to_ascii_lowercase();
            attach_tightly(&mut out, &format!(".{}", tld));
            i += 2;
            continue;
        }

        if url_mode && (!is_at(&words[i]) || email_at(words.get(i + 1), words.get(i + 2))) {
            if let Some(punct) = spoken_punctuation(&words[i]) {
                attach_tightly(&mut out, punct);
                tight_next = true;
                i += 1;
                continue;
            }
        }

        let mut value = words[i].clone();
        if capitalize_next {
            if !is_domain(&value) {
                value = capitalize_first(&value);
            }
            capitalize_next = false;
        }
        if tight_next {
            attach_tightly(&mut out, &value);
            tight_next = false;
        } else {
            attach(&mut out, &value);
        }
        i += 1;
    }
    if !out.is_empty() {
        state.mid_line = !out.ends_with('\n');
    }
    out
}

/// Marker for the item after `line` ("2. Milk" -> "3. ", "- Milk" -> "- "), read from the field
/// so deleted or inserted items never throw the count off. An empty item ends the list.
pub fn next_marker(line: &str) -> Option<String> {
    let line = line.trim_start();
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("• ")) {
        return (!rest.trim().is_empty()).then(|| "- ".to_string());
    }
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let rest = line[digits..].strip_prefix(". ")?;
    let n: u32 = line[..digits].parse().ok()?;
    (!rest.trim().is_empty()).then(|| format!("{}. ", n + 1))
}

/// Whether a line already starts with a list marker (the app continued the list itself).
pub fn has_marker(line: &str) -> bool {
    let line = line.trim_start();
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    line.starts_with("- ")
        || line.starts_with("• ")
        || (digits > 0 && line[digits..].starts_with('.'))
}

/// Whether the text speaks a list/line hook; S1-mini cleanup could rewrite or drop it.
pub fn has_hook(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    ["new line", "new paragraph", "bullet point", "number point"]
        .iter()
        .any(|hook| text.contains(hook))
}

/// Format one recognized dictation chunk. Rules are local and deterministic.
pub fn format_chunk(words: &str, url_mode: bool, state: &mut FormatState) -> String {
    format_tokens(words, url_mode, state)
}

/// Flush a held formatting word at utterance end or stop.
pub fn flush(state: &mut FormatState, url_mode: bool) -> String {
    let pending = std::mem::take(&mut state.pending);
    if pending.is_empty() {
        return String::new();
    }
    state.mid_line = true;
    if url_mode && !is_at(&pending) {
        if let Some(punct) = spoken_punctuation(&pending) {
            return punct.to_owned();
        }
    }
    pending
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_domains_and_url_speech() {
        let mut state = FormatState::default();
        assert_eq!(
            format_chunk("github dot com", false, &mut state),
            "github.com"
        );
        assert_eq!(
            format_chunk("Google.com.", false, &mut state),
            "google.com."
        );
        assert_eq!(
            format_chunk("I use Google.com. It works", false, &mut state),
            "I use google.com. It works"
        );
        assert_eq!(
            format_chunk("weather at noon", true, &mut state),
            "weather at noon"
        );
        assert_eq!(
            format_chunk("mail me at Acme.com", true, &mut state),
            "mail me@acme.com"
        );
        assert_eq!(format_chunk("john at", true, &mut state), "john");
        assert_eq!(
            format_chunk("gmail dot com", true, &mut state),
            "@gmail.com"
        );
        assert_eq!(format_chunk("meet at", true, &mut state), "meet");
        assert_eq!(format_chunk("noon", true, &mut state), "at noon");
        assert_eq!(
            format_chunk("polka dot dress", false, &mut state),
            "polka dot dress"
        );
        assert_eq!(
            format_chunk("docs dot rs slash serde", true, &mut state),
            "docs.rs/serde"
        );
        assert_eq!(
            format_chunk("john at gmail dot com", true, &mut state),
            "john@gmail.com"
        );
        assert_eq!(
            format_chunk("weather in boston.", true, &mut state),
            "weather in boston"
        );
        assert_eq!(
            format_chunk("Hello. there", true, &mut state),
            "Hello. there"
        );
    }

    #[test]
    fn formats_hooks_and_capitalization() {
        let mut state = FormatState::default();
        assert_eq!(
            format_chunk("eggs new line milk", false, &mut state),
            "eggs\nMilk"
        );
        assert_eq!(
            format_chunk("bullet point eggs bullet point milk", false, &mut state),
            "\n- Eggs\n- Milk"
        );
        // "number point" always starts a list; "new line" continues it while typing.
        assert_eq!(
            format_chunk("number point eggs new line milk", false, &mut state),
            "\n1. Eggs\nMilk"
        );
        assert_eq!(
            format_chunk("a new line of products", false, &mut state),
            "a new line of products"
        );
        assert_eq!(
            format_chunk("my bullet point", false, &mut state),
            "my bullet point"
        );
        assert_eq!(
            format_chunk("new paragraph number point eggs", false, &mut state),
            "\n\n1. Eggs"
        );
    }

    #[test]
    fn list_continues_from_the_fields_own_line() {
        assert_eq!(next_marker("2. Milk").as_deref(), Some("3. "));
        assert_eq!(next_marker("  9. Ninth").as_deref(), Some("10. "));
        assert_eq!(next_marker("- Eggs").as_deref(), Some("- "));
        assert_eq!(next_marker("3. "), None); // empty item ends the list
        assert_eq!(next_marker("Start."), None); // app hides its markers: leave it to the app
        assert_eq!(next_marker("2.5 kg of flour"), None);
        assert!(has_marker("2. ") && has_marker("- ") && !has_marker("Two"));
    }

    #[test]
    fn plain_dictation_is_untouched() {
        let mut state = FormatState::default();
        // Whisper's own punctuation must not stop hooks from matching.
        assert_eq!(
            format_chunk("Eggs, new line, milk.", false, &mut state),
            "Eggs,\nMilk."
        );
        // A missing space between sentences is not a domain.
        assert_eq!(
            format_chunk("Do it.In time", false, &mut state),
            "Do it.In time"
        );
        // "at" is only held back in URL mode.
        assert_eq!(format_chunk("meet me at", false, &mut state), "meet me at");
        assert!(state.pending.is_empty());
    }

    #[test]
    fn carries_a_trailing_dot_across_chunks() {
        let mut state = FormatState::default();
        assert_eq!(format_chunk("github dot", false, &mut state), "github");
        assert_eq!(format_chunk("com", false, &mut state), ".com");
    }

    #[test]
    fn carries_hook_prefixes_and_flushes_incomplete_markers_literally() {
        let mut state = FormatState::default();
        assert_eq!(format_chunk("eggs new", false, &mut state), "eggs");
        assert_eq!(format_chunk("line milk", false, &mut state), "\nMilk");

        assert_eq!(format_chunk("bullet", false, &mut state), "");
        assert_eq!(flush(&mut state, false), "bullet");
        assert_eq!(format_chunk("number", false, &mut state), "");
        assert_eq!(flush(&mut state, false), "number");
    }
}

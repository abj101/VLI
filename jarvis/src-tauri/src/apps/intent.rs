//! Deterministic open-intent parsing and app-vs-URL classification (Tier 0 preflight).

use super::{
    resolve_target::{is_explicit_url, strip_placement_suffix, top_app_candidates},
    AppEntry, APP_RESOLVE_MIN_RATIO,
};
use crate::db::{TargetAlias, TargetAliasKind};

#[derive(Debug, Clone, PartialEq)]
pub enum OpenIntent {
    App {
        target: String,
        display_name: String,
        exe_path: String,
        placement: Option<String>,
    },
    Url {
        url: String,
        placement: Option<String>,
    },
    Unknown {
        target: String,
    },
}

const OPEN_PREFIX_TRIGGERS: &[&str] = &["please open", "open"];

/// `(target, optional_placement)` when transcript is an open-prefix intent; `None` otherwise.
pub fn parse_open_intent(transcript: &str) -> Option<(String, Option<String>)> {
    let trimmed = transcript.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut triggers: Vec<&str> = OPEN_PREFIX_TRIGGERS.to_vec();
    triggers.sort_by_key(|t| std::cmp::Reverse(t.len()));
    for trigger in triggers {
        if let Some(end) = find_open_prefix_end(trimmed, trigger) {
            let remainder = trimmed[end..].trim();
            if remainder.is_empty() {
                return None;
            }
            let (target, placement) = strip_placement_suffix(remainder);
            let target = target.trim().to_string();
            if target.is_empty() {
                return None;
            }
            return Some((target, placement));
        }
    }
    None
}

/// App index lookup first, then explicit URL, then unknown (no silent `.com` guess).
pub fn classify_open_target(
    target: &str,
    placement: Option<String>,
    app_index: &[AppEntry],
    aliases: &[TargetAlias],
) -> OpenIntent {
    let (clean_target, suffix_placement) = strip_placement_suffix(target);
    let zone = placement.or(suffix_placement);
    let query = clean_target.trim();
    if query.is_empty() {
        return OpenIntent::Unknown {
            target: target.trim().to_string(),
        };
    }

    let spoken_key = query.to_lowercase();
    if let Some(alias) = aliases.iter().find(|a| a.spoken == spoken_key) {
        return match alias.kind {
            TargetAliasKind::App => OpenIntent::App {
                target: query.to_string(),
                display_name: alias.value.clone(),
                exe_path: alias.value.clone(),
                placement: zone,
            },
            TargetAliasKind::Url => OpenIntent::Url {
                url: alias.value.clone(),
                placement: zone,
            },
        };
    }

    let app_candidates = top_app_candidates(query, app_index, 1);
    if let Some(top) = app_candidates.first() {
        if top.score >= APP_RESOLVE_MIN_RATIO {
            return OpenIntent::App {
                target: query.to_string(),
                display_name: top.display_name.clone(),
                exe_path: top.exe_path.clone(),
                placement: zone,
            };
        }
    }

    if is_explicit_url(query) {
        return OpenIntent::Url {
            url: super::resolve_target::canonicalize_explicit_url(query),
            placement: zone,
        };
    }

    #[cfg(target_os = "macos")]
    if is_plausible_macos_app_name(query) {
        return OpenIntent::App {
            target: query.to_string(),
            display_name: query.to_string(),
            exe_path: query.to_string(),
            placement: zone,
        };
    }

    OpenIntent::Unknown {
        target: query.to_string(),
    }
}

fn find_open_prefix_end(transcript: &str, phrase: &str) -> Option<usize> {
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
                return Some(end);
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

fn prefix_match_byte_len_ci(hay: &str, needle: &str) -> Option<usize> {
    let mut len = 0usize;
    let mut hay_chars = hay.chars();
    for n in needle.chars() {
        let h = hay_chars.next()?;
        if !h.eq_ignore_ascii_case(&n) {
            return None;
        }
        len += h.len_utf8();
    }
    Some(len)
}

#[cfg(target_os = "macos")]
pub(crate) fn is_plausible_macos_app_name(query: &str) -> bool {
    let t = query.trim();
    if t.is_empty() || t.len() > 64 {
        return false;
    }
    if t.contains("://") || t.contains('.') {
        return false;
    }
    t.chars()
        .all(|c| c.is_alphanumeric() || c.is_whitespace() || c == '-' || c == '_' || c == '+')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::resolve_target::is_explicit_url;
    use crate::db::{TargetAlias, TargetAliasKind};

    fn brave_entry() -> AppEntry {
        AppEntry {
            display_name: "Brave".into(),
            exe_path: r"C:\Brave\brave.exe".into(),
            icon_data_url: None,
        }
    }

    #[test]
    fn is_explicit_url_accepts_domains_and_schemes() {
        assert!(is_explicit_url("google.com"));
        assert!(is_explicit_url("https://slack.com"));
        assert!(is_explicit_url("www.reddit.com"));
        assert!(is_explicit_url("google dot com"));
    }

    #[test]
    fn is_explicit_url_rejects_bare_app_names() {
        assert!(!is_explicit_url("slack"));
        assert!(!is_explicit_url("brave"));
        assert!(!is_explicit_url("github"));
    }

    #[test]
    fn parse_open_intent_extracts_target_and_placement() {
        let (target, placement) = parse_open_intent("open brave on the left").expect("parsed");
        assert_eq!(target, "brave");
        assert_eq!(placement.as_deref(), Some("left_half"));
    }

    #[test]
    fn parse_open_intent_rejects_opening_without_boundary() {
        assert!(parse_open_intent("opening notepad").is_none());
    }

    #[test]
    fn parse_open_intent_accepts_leading_please() {
        let (target, _) = parse_open_intent("please open notepad").expect("parsed");
        assert_eq!(target, "notepad");
    }

    #[test]
    fn classify_open_target_prefers_app_over_url_guess() {
        let entries = vec![brave_entry()];
        let intent = classify_open_target("brave", None, &entries, &[]);
        assert_eq!(
            intent,
            OpenIntent::App {
                target: "brave".into(),
                display_name: "Brave".into(),
                exe_path: brave_entry().exe_path,
                placement: None,
            }
        );
    }

    #[test]
    fn classify_open_target_unknown_when_not_in_index() {
        let intent = classify_open_target("foobar", None, &[], &[]);
        #[cfg(target_os = "macos")]
        assert!(matches!(intent, OpenIntent::App { .. }));
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            intent,
            OpenIntent::Unknown {
                target: "foobar".into()
            }
        );
    }

    #[test]
    fn classify_open_target_url_for_explicit_domain() {
        let intent = classify_open_target("google.com", None, &[], &[]);
        assert_eq!(
            intent,
            OpenIntent::Url {
                url: "https://google.com".into(),
                placement: None,
            }
        );
    }

    #[test]
    fn classify_open_target_github_app_beats_bare_name() {
        let entries = vec![AppEntry {
            display_name: "GitHub".into(),
            exe_path: "GitHubDesktop.exe".into(),
            icon_data_url: None,
        }];
        let intent = classify_open_target("github", None, &entries, &[]);
        assert!(matches!(intent, OpenIntent::App { .. }));
    }

    #[test]
    fn classify_open_target_respects_url_alias() {
        let aliases = vec![TargetAlias {
            spoken: "github".into(),
            kind: TargetAliasKind::Url,
            value: "https://github.com".into(),
        }];
        let intent = classify_open_target("github", None, &[], &aliases);
        assert_eq!(
            intent,
            OpenIntent::Url {
                url: "https://github.com".into(),
                placement: None,
            }
        );
    }
}

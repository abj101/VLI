//! Smart app vs URL resolution for the `open_target` builtin tool.

use super::{AppEntry, APP_RESOLVE_MIN_RATIO};
use crate::db::TargetAlias;
use rapidfuzz::fuzz;
use serde::Serialize;

/// How many app index entries to retain for ambiguous clarify flows.
pub const TOP_APP_CANDIDATES: usize = 5;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppCandidate {
    pub display_name: String,
    pub exe_path: String,
    pub score: f64,
}

impl AppCandidate {
    fn from_entry(entry: &AppEntry, score: f64) -> Self {
        Self {
            display_name: entry.display_name.clone(),
            exe_path: entry.exe_path.clone(),
            score,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UrlCandidate {
    pub url: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedTarget {
    App {
        display_name: String,
        exe_path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        placement: Option<String>,
    },
    Url {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        placement: Option<String>,
    },
    Ambiguous {
        query: String,
        app_candidates: Vec<AppCandidate>,
        url_candidate: UrlCandidate,
        #[serde(skip_serializing_if = "Option::is_none")]
        placement: Option<String>,
    },
    Unknown {
        query: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        placement: Option<String>,
    },
}

/// Collapse whitespace and drop trailing STT punctuation before suffix matching.
fn normalize_for_placement_strip(input: &str) -> String {
    let trimmed = input.trim();
    let no_trail_punct = trimmed.trim_end_matches(|c: char| {
        matches!(c, '.' | ',' | '!' | '?' | ';' | ':' | '…')
    });
    no_trail_punct
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `(clean_target, optional_placement_zone)`.
pub fn strip_placement_suffix(input: &str) -> (String, Option<String>) {
    let normalized = normalize_for_placement_strip(input);
    if normalized.is_empty() {
        return (String::new(), None);
    }
    let lower = normalized.to_lowercase();
    let mut suffixes: Vec<(&str, &str)> = PLACEMENT_SUFFIXES.to_vec();
    suffixes.sort_by_key(|(phrase, _)| std::cmp::Reverse(phrase.len()));
    for (phrase, zone) in suffixes {
        if lower == phrase {
            return (String::new(), Some(zone.to_string()));
        }
        let prefix = format!(" {phrase}");
        if lower.ends_with(&prefix) {
            let byte_end = normalized.len().saturating_sub(prefix.len());
            let clean = normalized[..byte_end].trim();
            if !clean.is_empty() {
                return (clean.to_string(), Some(zone.to_string()));
            }
        }
    }
    (normalized, None)
}

const PLACEMENT_SUFFIXES: &[(&str, &str)] = &[
    ("on the left side", "left_half"),
    ("on the right side", "right_half"),
    ("to the left", "left_half"),
    ("to the right", "right_half"),
    ("on the left", "left_half"),
    ("on the right", "right_half"),
    ("on left", "left_half"),
    ("on right", "right_half"),
    ("snap left", "left_half"),
    ("snap right", "right_half"),
    ("left half", "left_half"),
    ("right half", "right_half"),
    ("full screen", "maximize"),
    ("fullscreen", "maximize"),
    ("maximize", "maximize"),
];

/// Map composer / spoken placement labels to executor zone ids.
pub fn normalize_placement_zone(zone: &str) -> Option<String> {
    match zone.trim().to_lowercase().as_str() {
        "" => None,
        "left" | "left_half" | "left half" | "on the left" | "snap left" => {
            Some("left_half".into())
        }
        "right" | "right_half" | "right half" | "on the right" | "snap right" => {
            Some("right_half".into())
        }
        "fullscreen" | "full screen" | "full_screen" | "maximize" | "maximized" => {
            Some("maximize".into())
        }
        other => Some(other.to_string()),
    }
}

/// Known spoken names → canonical HTTPS URLs (checked before the `.com` heuristic).
const KNOWN_SITES: &[(&str, &str)] = &[
    ("github", "https://github.com"),
    ("youtube", "https://youtube.com"),
    ("google", "https://google.com"),
    ("gmail", "https://mail.google.com"),
    ("reddit", "https://reddit.com"),
    ("twitter", "https://x.com"),
    ("x", "https://x.com"),
    ("facebook", "https://facebook.com"),
    ("instagram", "https://instagram.com"),
    ("linkedin", "https://linkedin.com"),
    ("stackoverflow", "https://stackoverflow.com"),
    ("amazon", "https://amazon.com"),
    ("netflix", "https://netflix.com"),
    ("twitch", "https://twitch.tv"),
    ("discord", "https://discord.com"),
    ("spotify", "https://open.spotify.com"),
    ("wikipedia", "https://wikipedia.org"),
];

pub fn resolve_target(
    input: &str,
    app_entries: &[AppEntry],
    aliases: &[TargetAlias],
) -> ResolvedTarget {
    let (clean_target, stripped_placement) = strip_placement_suffix(input);
    let query = clean_target.trim();
    let placement = stripped_placement;
    if query.is_empty() {
        return ResolvedTarget::Ambiguous {
            query: input.trim().to_string(),
            app_candidates: vec![],
            url_candidate: UrlCandidate {
                url: String::new(),
                score: 0.0,
            },
            placement,
        };
    }

    let spoken_key = query.to_lowercase();
    if let Some(alias) = aliases.iter().find(|a| a.spoken == spoken_key) {
        return alias_to_resolved(alias, placement);
    }

    let app_candidates = top_app_candidates(query, app_entries, TOP_APP_CANDIDATES);
    let app_score = app_candidates.first().map(|c| c.score).unwrap_or(0.0);

    if app_score >= APP_RESOLVE_MIN_RATIO {
        let top = app_candidates.first().expect("app_wins implies candidate");
        return ResolvedTarget::App {
            display_name: top.display_name.clone(),
            exe_path: top.exe_path.clone(),
            placement,
        };
    }

    if is_explicit_url(query) {
        return ResolvedTarget::Url {
            url: canonicalize_explicit_url(query),
            placement,
        };
    }

    if !app_candidates.is_empty() {
        let url_candidate = score_url_candidate_for_clarify(query);
        return ResolvedTarget::Ambiguous {
            query: query.to_string(),
            app_candidates,
            url_candidate: url_candidate.unwrap_or(UrlCandidate {
                url: String::new(),
                score: 0.0,
            }),
            placement,
        };
    }

    ResolvedTarget::Unknown {
        query: query.to_string(),
        placement,
    }
}

fn alias_to_resolved(alias: &TargetAlias, placement: Option<String>) -> ResolvedTarget {
    match alias.kind {
        crate::db::TargetAliasKind::App => ResolvedTarget::App {
            display_name: alias.value.clone(),
            exe_path: alias.value.clone(),
            placement,
        },
        crate::db::TargetAliasKind::Url => ResolvedTarget::Url {
            url: alias.value.clone(),
            placement,
        },
    }
}

pub fn top_app_candidates(
    query: &str,
    entries: &[AppEntry],
    limit: usize,
) -> Vec<AppCandidate> {
    let q_lower = query.trim().to_lowercase();
    if q_lower.is_empty() {
        return vec![];
    }
    let mut scored: Vec<AppCandidate> = entries
        .iter()
        .map(|e| AppCandidate::from_entry(e, score_entry(&q_lower, e)))
        .filter(|c| c.score > 0.0)
        .collect();
    scored.sort_by(|a, b| {
        match b
            .score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
        {
            std::cmp::Ordering::Equal => {
                super::prefer_launch_path(&a.exe_path, &b.exe_path)
            }
            ord => ord,
        }
    });
    scored.truncate(limit);
    scored
}

fn score_entry(query_lower: &str, e: &AppEntry) -> f64 {
    let name = fuzz::ratio(
        query_lower.chars(),
        e.display_name.to_lowercase().chars(),
    );
    let stem = std::path::Path::new(&e.exe_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| fuzz::ratio(query_lower.chars(), s.to_lowercase().chars()))
        .unwrap_or(0.0);
    name.max(stem)
}

/// True when `text` looks like an explicit URL or spoken domain (not a bare app name).
pub fn is_explicit_url(text: &str) -> bool {
    let normalized = normalize_spoken_url(text);
    let t = normalized.trim().to_lowercase();
    if t.is_empty() {
        return false;
    }
    if t.contains("http://") || t.contains("https://") || t.contains("www.") {
        return true;
    }
    has_domain_tld_pattern(&t)
}

pub(crate) fn canonicalize_explicit_url(text: &str) -> String {
    let normalized = normalize_spoken_url(text);
    let t = normalized.trim().to_lowercase();
    if t.starts_with("http://") || t.starts_with("https://") {
        t
    } else {
        format!("https://{t}")
    }
}

fn normalize_spoken_url(text: &str) -> String {
    let mut s = text.to_lowercase();
    for (spoken, dot) in [
        (" dot com", ".com"),
        (" dot org", ".org"),
        (" dot net", ".net"),
        (" dot edu", ".edu"),
        (" dot io", ".io"),
        (" dot co uk", ".co.uk"),
        (" dot co", ".co"),
        (" dot uk", ".uk"),
    ] {
        s = s.replace(spoken, dot);
    }
    s
}

fn has_domain_tld_pattern(t: &str) -> bool {
    if !t.contains('.') {
        return false;
    }
    let host = t.split('/').next().unwrap_or(t).trim();
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    let tld = parts.last().unwrap_or(&"");
    tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()) && !parts[0].is_empty()
}

fn score_url_candidate_for_clarify(target: &str) -> Option<UrlCandidate> {
    let t = target.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }

    let mut best: Option<UrlCandidate> = None;
    for (key, url) in KNOWN_SITES {
        let score = fuzz::ratio(t.chars(), key.chars());
        if score < 0.85 {
            continue;
        }
        let normalized = score.max(0.9);
        let replace = match &best {
            None => true,
            Some(prev) => normalized > prev.score + 1e-9,
        };
        if replace {
            best = Some(UrlCandidate {
                url: (*url).to_string(),
                score: normalized,
            });
        }
    }
    if best.is_some() {
        return best;
    }

    if is_explicit_url(target) {
        return Some(UrlCandidate {
            url: canonicalize_explicit_url(target),
            score: 0.9,
        });
    }

    None
}

/// Map a clarify follow-up utterance to app vs browser.
pub fn interpret_clarify_choice(response: &str, ambiguous: &ResolvedTarget) -> Option<bool> {
    let ResolvedTarget::Ambiguous { .. } = ambiguous else {
        return None;
    };
    let r = response.trim().to_lowercase();
    if r.is_empty() {
        return None;
    }
    if r.contains("browser")
        || r.contains("website")
        || r.contains("web")
        || r.contains("url")
        || r.contains("site")
        || r.contains("internet")
    {
        return Some(false);
    }
    if r.contains("app")
        || r.contains("application")
        || r.contains("program")
        || r.contains("desktop")
    {
        return Some(true);
    }
    if let ResolvedTarget::Ambiguous {
        app_candidates, ..
    } = ambiguous
    {
        for candidate in app_candidates {
            let name = candidate.display_name.to_lowercase();
            if r.contains(&name) || name.contains(&r) {
                return Some(true);
            }
        }
    }
    None
}

pub fn ambiguous_to_choice(
    ambiguous: &ResolvedTarget,
    choose_app: bool,
) -> Option<ResolvedTarget> {
    let ResolvedTarget::Ambiguous {
        query,
        app_candidates,
        url_candidate,
        placement,
    } = ambiguous
    else {
        return None;
    };
    if choose_app {
        let top = app_candidates.first()?;
        Some(ResolvedTarget::App {
            display_name: top.display_name.clone(),
            exe_path: top.exe_path.clone(),
            placement: placement.clone(),
        })
    } else {
        Some(ResolvedTarget::Url {
            url: if url_candidate.url.is_empty() {
                format!("https://{}.com", query.to_lowercase())
            } else {
                url_candidate.url.clone()
            },
            placement: placement.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{TargetAlias, TargetAliasKind};

    fn brave_entry() -> AppEntry {
        AppEntry {
            display_name: "Brave".into(),
            exe_path: r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe".into(),
            icon_data_url: None,
        }
    }

    fn github_app_entry() -> AppEntry {
        AppEntry {
            display_name: "GitHub".into(),
            exe_path: r"C:\Users\x\AppData\Local\GitHubDesktop\GitHubDesktop.exe".into(),
            icon_data_url: None,
        }
    }

    #[test]
    fn normalize_placement_zone_maps_fullscreen() {
        assert_eq!(
            normalize_placement_zone("fullscreen").as_deref(),
            Some("maximize")
        );
        assert_eq!(
            normalize_placement_zone("full screen").as_deref(),
            Some("maximize")
        );
    }

    #[test]
    fn strip_placement_suffix_removes_trailing_left_phrase() {
        let (clean, placement) = strip_placement_suffix("brave on the left");
        assert_eq!(clean, "brave");
        assert_eq!(placement.as_deref(), Some("left_half"));
    }

    #[test]
    fn strip_placement_suffix_leaves_bare_target() {
        let (clean, placement) = strip_placement_suffix("  Brave  ");
        assert_eq!(clean, "Brave");
        assert!(placement.is_none());
    }

    #[test]
    fn strip_placement_suffix_ignores_trailing_stt_punctuation() {
        let (clean, placement) = strip_placement_suffix("notepad on the left.");
        assert_eq!(clean, "notepad");
        assert_eq!(placement.as_deref(), Some("left_half"));
    }

    #[test]
    fn strip_placement_suffix_collapses_extra_whitespace() {
        let (clean, placement) = strip_placement_suffix("brave on  the left");
        assert_eq!(clean, "brave");
        assert_eq!(placement.as_deref(), Some("left_half"));
    }

    #[test]
    fn resolve_target_notepad_on_the_left_prefers_app() {
        let entries = vec![AppEntry {
            display_name: "Notepad".into(),
            exe_path: "notepad.exe".into(),
            icon_data_url: None,
        }];
        let resolved = resolve_target("notepad on the left.", &entries, &[]);
        assert_eq!(
            resolved,
            ResolvedTarget::App {
                display_name: "Notepad".into(),
                exe_path: "notepad.exe".into(),
                placement: Some("left_half".into()),
            }
        );
    }

    #[test]
    fn resolve_target_brave_prefers_app() {
        let entries = vec![brave_entry()];
        let resolved = resolve_target("brave", &entries, &[]);
        assert_eq!(
            resolved,
            ResolvedTarget::App {
                display_name: "Brave".into(),
                exe_path: brave_entry().exe_path,
                placement: None,
            }
        );
    }

    #[test]
    fn resolve_target_github_prefers_app_without_alias() {
        let entries = vec![github_app_entry()];
        let resolved = resolve_target("github", &entries, &[]);
        assert_eq!(
            resolved,
            ResolvedTarget::App {
                display_name: "GitHub".into(),
                exe_path: github_app_entry().exe_path,
                placement: None,
            }
        );
    }

    #[test]
    fn resolve_target_foobar_unknown_without_com_guess() {
        let resolved = resolve_target("foobar", &[], &[]);
        assert_eq!(
            resolved,
            ResolvedTarget::Unknown {
                query: "foobar".into(),
                placement: None,
            }
        );
    }

    #[test]
    fn resolve_target_google_dot_com_is_url() {
        let resolved = resolve_target("google.com", &[], &[]);
        assert_eq!(
            resolved,
            ResolvedTarget::Url {
                url: "https://google.com".into(),
                placement: None,
            }
        );
    }

    #[test]
    fn resolve_target_learned_alias_used_on_second_resolve() {
        let entries = vec![github_app_entry()];
        let aliases = vec![TargetAlias {
            spoken: "github".into(),
            kind: TargetAliasKind::Url,
            value: "https://github.com".into(),
        }];
        let first = resolve_target("github", &entries, &[]);
        assert!(matches!(first, ResolvedTarget::App { .. }));
        let second = resolve_target("github", &entries, &aliases);
        assert_eq!(
            second,
            ResolvedTarget::Url {
                url: "https://github.com".into(),
                placement: None,
            }
        );
    }

    #[test]
    fn resolve_target_alias_github_url_overrides_app() {
        let entries = vec![github_app_entry()];
        let aliases = vec![TargetAlias {
            spoken: "github".into(),
            kind: TargetAliasKind::Url,
            value: "https://github.com".into(),
        }];
        let resolved = resolve_target("github", &entries, &aliases);
        assert_eq!(
            resolved,
            ResolvedTarget::Url {
                url: "https://github.com".into(),
                placement: None,
            }
        );
    }

    #[test]
    fn resolve_target_strips_placement_before_scoring() {
        let entries = vec![brave_entry()];
        let resolved = resolve_target("brave snap right", &entries, &[]);
        assert_eq!(
            resolved,
            ResolvedTarget::App {
                display_name: "Brave".into(),
                exe_path: brave_entry().exe_path,
                placement: Some("right_half".into()),
            }
        );
    }

    #[test]
    fn interpret_clarify_choice_browser_keywords() {
        let ambiguous = ResolvedTarget::Ambiguous {
            query: "github".into(),
            app_candidates: vec![AppCandidate {
                display_name: "GitHub".into(),
                exe_path: "gh.exe".into(),
                score: 1.0,
            }],
            url_candidate: UrlCandidate {
                url: "https://github.com".into(),
                score: 1.0,
            },
            placement: None,
        };
        assert_eq!(interpret_clarify_choice("browser", &ambiguous), Some(false));
        assert_eq!(interpret_clarify_choice("the app", &ambiguous), Some(true));
    }

    #[test]
    fn top_app_candidates_orders_by_score() {
        let entries = vec![
            AppEntry {
                display_name: "Brave".into(),
                exe_path: "brave.exe".into(),
                icon_data_url: None,
            },
            AppEntry {
                display_name: "Bravo Studio".into(),
                exe_path: "bravo.exe".into(),
                icon_data_url: None,
            },
        ];
        let hits = top_app_candidates("brave", &entries, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].display_name, "Brave");
        assert!(hits[0].score >= hits[1].score);
    }
}

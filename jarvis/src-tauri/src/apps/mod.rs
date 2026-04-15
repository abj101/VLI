//! Installed-app index (Windows): scan → cache → fuzzy resolve for `OpenApp` without a path.

#[cfg(windows)]
mod scanner_windows;

use rapidfuzz::fuzz;
use std::path::Path;

/// Minimum [`fuzz::ratio`] (normalized 0..=1) for a display name / exe stem to count as a match (Phase 4: 0.75).
pub const APP_RESOLVE_MIN_RATIO: f64 = 0.75;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct AppEntry {
    pub display_name: String,
    pub exe_path: String,
}

/// Best fuzzy match at or above [`APP_RESOLVE_MIN_RATIO`], comparing `query` to display name and to the exe file stem.
pub fn resolve_app<'a>(query: &str, entries: &'a [AppEntry]) -> Option<&'a AppEntry> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    let q_lower = q.to_lowercase();
    let mut best: Option<(f64, &'a AppEntry)> = None;
    for e in entries {
        let r = score_entry(&q_lower, e);
        if r + f64::EPSILON < APP_RESOLVE_MIN_RATIO {
            continue;
        }
        let replace = match &best {
            None => true,
            Some((score, _)) => r > *score + 1e-9,
        };
        if replace {
            best = Some((r, e));
        }
    }
    best.map(|(_, e)| e)
}

fn score_entry(query_lower: &str, e: &AppEntry) -> f64 {
    let name = fuzz::ratio(query_lower.chars(), e.display_name.to_lowercase().chars());
    let stem = Path::new(&e.exe_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| fuzz::ratio(query_lower.chars(), s.to_lowercase().chars()))
        .unwrap_or(0.0);
    name.max(stem)
}

/// Full scan (registry + Start Menu shortcuts on Windows; empty elsewhere).
pub fn scan_installed_apps() -> Vec<AppEntry> {
    #[cfg(windows)]
    {
        match scanner_windows::scan() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("app index scan failed: {e}");
                Vec::new()
            }
        }
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_app_picks_over_threshold() {
        let entries = vec![
            AppEntry {
                display_name: "Discord".into(),
                exe_path: r"C:\Apps\Discord.exe".into(),
            },
            AppEntry {
                display_name: "Notepad".into(),
                exe_path: r"C:\Windows\notepad.exe".into(),
            },
        ];
        let hit = resolve_app("discrod", &entries).expect("fuzzy match");
        assert_eq!(hit.display_name, "Discord");
    }

    #[test]
    fn resolve_app_none_below_threshold() {
        let entries = vec![AppEntry {
            display_name: "Zebra Viewer 9000".into(),
            exe_path: r"C:\z.exe".into(),
        }];
        assert!(resolve_app("completely different thing", &entries).is_none());
    }

    #[test]
    fn resolve_app_matches_exe_stem() {
        let entries = vec![AppEntry {
            display_name: "X".into(),
            exe_path: r"C:\Stuff\Microsoft Edge.exe".into(),
        }];
        let hit = resolve_app("Microsoft Edge", &entries).expect("stem match");
        assert!(hit.exe_path.contains("Edge"));
    }
}

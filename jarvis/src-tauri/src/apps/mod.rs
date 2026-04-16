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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_data_url: Option<String>,
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
        if !picker_indexable(e) {
            continue;
        }
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

/// Picker + browse lists hide the same junk the scanner refuses to insert
/// (`Update.exe`, `*SystemHelper.exe`, …) so an old SQLite cache cannot
/// resurrect them until the next full rescan.
#[inline]
fn picker_indexable(e: &AppEntry) -> bool {
    #[cfg(windows)]
    {
        scanner_windows::should_index_launch_target(&e.exe_path)
    }
    #[cfg(not(windows))]
    {
        !e.exe_path.trim().is_empty()
    }
}

/// First `limit` entries sorted by display name (case-insensitive), then `exe_path`.
pub fn sorted_app_name_slice(entries: &[AppEntry], limit: usize) -> Vec<AppEntry> {
    let mut idx: Vec<usize> = (0..entries.len())
        .filter(|&i| picker_indexable(&entries[i]))
        .collect();
    idx.sort_by(|&i, &j| {
        entries[i]
            .display_name
            .to_lowercase()
            .cmp(&entries[j].display_name.to_lowercase())
            .then_with(|| entries[i].exe_path.cmp(&entries[j].exe_path))
    });
    idx.into_iter()
        .take(limit)
        .map(|i| entries[i].clone())
        .collect()
}

/// Substring match on display name, full path, and exe stem (`query_lower` = trimmed + lowercase).
pub fn filter_app_entries_substring(entries: &[AppEntry], query_lower: &str, limit: usize) -> Vec<AppEntry> {
    let mut matched: Vec<AppEntry> = entries
        .iter()
        .filter(|e| picker_indexable(e))
        .filter(|e| {
            let name = e.display_name.to_lowercase();
            let path = e.exe_path.to_lowercase();
            let stem = Path::new(&e.exe_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            name.contains(query_lower)
                || path.contains(query_lower)
                || stem.contains(query_lower)
        })
        .cloned()
        .collect();
    matched.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
            .then_with(|| a.exe_path.cmp(&b.exe_path))
    });
    matched.truncate(limit);
    matched
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

/// Lazy icon extraction — called by the `get_app_icon` Tauri command when the
/// UI renders a confirmed pick. Kept out of the hot scan path so `scan()`
/// returns in seconds even on machines with hundreds of installed apps.
pub fn get_app_icon(exe_path: &str) -> Option<String> {
    if exe_path.trim().is_empty() {
        return None;
    }
    #[cfg(windows)]
    {
        scanner_windows::extract_icon_data_url(exe_path)
    }
    #[cfg(not(windows))]
    {
        None
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
                icon_data_url: None,
            },
            AppEntry {
                display_name: "Notepad".into(),
                exe_path: r"C:\Windows\notepad.exe".into(),
                icon_data_url: None,
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
            icon_data_url: None,
        }];
        assert!(resolve_app("completely different thing", &entries).is_none());
    }

    #[test]
    fn resolve_app_matches_exe_stem() {
        let entries = vec![AppEntry {
            display_name: "X".into(),
            exe_path: r"C:\Stuff\Microsoft Edge.exe".into(),
            icon_data_url: None,
        }];
        let hit = resolve_app("Microsoft Edge", &entries).expect("stem match");
        assert!(hit.exe_path.contains("Edge"));
    }

    #[test]
    fn app_entry_serializes_icon_data_url_when_present() {
        let entry = AppEntry {
            display_name: "Notepad".into(),
            exe_path: r"C:\Windows\notepad.exe".into(),
            icon_data_url: Some("data:image/png;base64,AAA=".into()),
        };
        let value = serde_json::to_value(entry).expect("serialize app entry");
        assert_eq!(
            value.get("icon_data_url").and_then(|v| v.as_str()),
            Some("data:image/png;base64,AAA=")
        );
    }

    #[test]
    fn sorted_app_name_slice_is_alphabetical_capped() {
        let entries = vec![
            AppEntry {
                display_name: "Zebra".into(),
                exe_path: r"C:\z.exe".into(),
                icon_data_url: None,
            },
            AppEntry {
                display_name: "Alpha".into(),
                exe_path: r"C:\a.exe".into(),
                icon_data_url: None,
            },
        ];
        let got = sorted_app_name_slice(&entries, 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].display_name, "Alpha");
    }

    #[test]
    fn filter_finds_notepad_discord_and_cs2_fixtures() {
        let entries = vec![
            AppEntry {
                display_name: "Notepad".into(),
                exe_path: r"C:\Windows\System32\notepad.exe".into(),
                icon_data_url: None,
            },
            AppEntry {
                display_name: "Discord".into(),
                exe_path: r"C:\Users\x\AppData\Local\Discord\app-1.0\Discord.exe".into(),
                icon_data_url: None,
            },
            AppEntry {
                display_name: "Counter-Strike 2".into(),
                exe_path: r"D:\Steam\steamapps\common\Counter-Strike Global Offensive\game\bin\win64\cs2.exe"
                    .into(),
                icon_data_url: None,
            },
        ];
        assert!(
            !filter_app_entries_substring(&entries, "notepad", 24).is_empty(),
            "notepad"
        );
        assert!(
            !filter_app_entries_substring(&entries, "discord", 24).is_empty(),
            "discord"
        );
        assert!(
            !filter_app_entries_substring(&entries, "counter", 24).is_empty(),
            "counter / CS2"
        );
        assert!(
            !filter_app_entries_substring(&entries, "cs2", 24).is_empty(),
            "cs2 stem"
        );
    }

    #[cfg(windows)]
    #[test]
    fn filter_and_sorted_slice_drop_squirrel_junk_exes() {
        let entries = vec![
            AppEntry {
                display_name: "Discord".into(),
                exe_path: r"C:\Users\x\AppData\Local\Discord\Update.exe".into(),
                icon_data_url: None,
            },
            AppEntry {
                display_name: "Discord".into(),
                exe_path: r"C:\Users\x\AppData\Local\Discord\app-1.0\Discord.exe".into(),
                icon_data_url: None,
            },
        ];
        let hits = filter_app_entries_substring(&entries, "discord", 10);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].exe_path.to_lowercase().ends_with("discord.exe"));
        let slice = sorted_app_name_slice(&entries, 10);
        assert_eq!(slice.len(), 1);
    }
}

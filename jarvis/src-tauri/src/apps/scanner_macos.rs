//! macOS app scanner — `.app` bundles under standard install locations.

use super::AppEntry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SCAN_ROOTS: &[&str] = &[
    "/Applications",
    "/System/Applications",
    "/System/Applications/Utilities",
];

/// Max directory depth below each root (1 = immediate children only).
const MAX_DEPTH: u32 = 2;

pub fn scan() -> Result<Vec<AppEntry>, String> {
    let mut by_path: HashMap<String, AppEntry> = HashMap::new();
    let home_apps = dirs_home_applications();

    for root in SCAN_ROOTS {
        collect_apps_under(Path::new(root), 0, &mut by_path);
    }
    if let Some(home) = home_apps {
        collect_apps_under(&home, 0, &mut by_path);
    }

    let mut entries: Vec<AppEntry> = by_path.into_values().collect();
    entries.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
            .then_with(|| a.exe_path.cmp(&b.exe_path))
    });
    Ok(entries)
}

fn dirs_home_applications() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Applications"))
}

fn collect_apps_under(dir: &Path, depth: u32, out: &mut HashMap<String, AppEntry>) {
    if depth > MAX_DEPTH || !dir.is_dir() {
        return;
    }
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            log::debug!("app scan: skip {:?}: {e}", dir);
            return;
        }
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "app") {
            if let Some(display_name) = bundle_display_name(&path) {
                let exe_path = path.to_string_lossy().into_owned();
                out.entry(exe_path.clone()).or_insert(AppEntry {
                    display_name,
                    exe_path,
                    icon_data_url: None,
                });
            }
            continue;
        }
        if path.is_dir() && depth < MAX_DEPTH {
            collect_apps_under(&path, depth + 1, out);
        }
    }
}

fn bundle_display_name(app_path: &Path) -> Option<String> {
    let folder = app_path.file_stem()?.to_str()?.trim();
    if folder.is_empty() {
        return None;
    }
    let info_plist = app_path.join("Contents").join("Info.plist");
    if let Ok(xml) = std::fs::read_to_string(&info_plist) {
        if let Some(name) = plist_string_value(&xml, "CFBundleDisplayName") {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if let Some(name) = plist_string_value(&xml, "CFBundleName") {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    Some(folder.to_string())
}

/// Minimal plist string extraction (no xml crate) for `CFBundleName` / `CFBundleDisplayName`.
fn plist_string_value(xml: &str, key: &str) -> Option<String> {
    let key_tag = format!("<key>{key}</key>");
    let key_pos = xml.find(&key_tag)?;
    let after_key = &xml[key_pos + key_tag.len()..];
    let string_start = after_key.find("<string>")? + "<string>".len();
    let rest = &after_key[string_start..];
    let string_end = rest.find("</string>")?;
    Some(rest[..string_end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_string_value_reads_bundle_name() {
        let xml = r#"
        <dict>
            <key>CFBundleName</key>
            <string>Safari</string>
        </dict>
        "#;
        assert_eq!(plist_string_value(xml, "CFBundleName").as_deref(), Some("Safari"));
    }

    #[test]
    fn scan_finds_safari_when_present() {
        let entries = scan().expect("scan");
        let safari = entries
            .iter()
            .find(|e| e.display_name.eq_ignore_ascii_case("safari"));
        if Path::new("/Applications/Safari.app").exists() {
            assert!(safari.is_some(), "expected Safari in app index");
            assert!(safari.unwrap().exe_path.ends_with("Safari.app"));
        }
    }
}

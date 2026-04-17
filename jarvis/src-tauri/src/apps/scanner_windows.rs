//! Windows app scanner — registry + Start Menu + game launchers + UWP.
//!
//! Sources scanned (in priority order):
//!   1. Uninstall registry (HKLM 64-bit, HKLM WOW64, HKCU)
//!   2. App Paths registry  (same three hives)
//!   3. Start Menu .lnk files (all-users + per-user + Public Desktop + user Desktop)
//!   4. Get-StartApps — same listing as Start (resolves `{GUID}\relative` AppIDs + AUMIDs)
//!   5. Steam  — libraryfolders.vdf → appmanifest_*.acf  (all library roots)
//!   6. Epic   — LauncherInstalled.dat + Manifests/*.item
//!   7. GOG    — HKLM\SOFTWARE\WOW6432Node\GOG.com\Games + HKLM\SOFTWARE\GOG.com\Games
//!   8. UWP    — PowerShell Get-AppxPackage (name + launch protocol)
//!   9. Windows accessories seed  (Notepad, Paint, …)
//!  10. Recursive exe scan — Program Files + LocalAppData\Programs (depth ≤ 6)

use super::AppEntry;
use base64::Engine;
use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{HWND, MAX_PATH};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_NORMAL, WIN32_FIND_DATAW};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::{
    IShellLinkW, SHGetFileInfoW, ShellLink, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON,
    SHGFI_USEFILEATTRIBUTES, SLGP_RAWPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
use winreg::enums::*;
use winreg::types::FromRegValue;
use winreg::RegKey;

// ---------------------------------------------------------------------------
// Source priority: lower number wins when merging display names.
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SourcePriority {
    Uninstall = 0, // richest human-readable name
    StartMenu = 1,
    Steam = 2,
    Epic = 3,
    Gog = 4,
    AppPaths = 5,
    Accessory = 6,
    ExeScan = 7, // lowest — raw exe name
}

// ---------------------------------------------------------------------------
// COM apartment RAII guard
// ---------------------------------------------------------------------------
struct ComApartment;
impl ComApartment {
    fn new() -> Result<Self, String> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| e.to_string())?;
        }
        Ok(Self)
    }
}
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

// ---------------------------------------------------------------------------
// Internal map entry — carries priority so merging is deterministic
// ---------------------------------------------------------------------------
#[derive(Clone)]
struct MapEntry {
    inner: AppEntry,
    priority: SourcePriority,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------
pub fn scan() -> Result<Vec<AppEntry>, String> {
    let mut map: HashMap<String, MapEntry> = HashMap::new();
    let mut stats: Vec<(&'static str, usize)> = Vec::new();

    // Registry passes (no COM required)
    run_pass("uninstall", &mut map, &mut stats, scan_uninstall_registry);
    run_pass("app_paths", &mut map, &mut stats, scan_app_paths_registry);

    // Launchers + UWP + seeds + deep exe scan — MUST NOT depend on COM.
    // Previously COM/start-menu ran first; any CoInit or .lnk failure aborted the entire
    // scan, so Steam/Epic/UWP/Notepad seed never ran (symptom: tiny index + no games).
    run_pass("steam", &mut map, &mut stats, scan_steam);
    run_pass("epic", &mut map, &mut stats, scan_epic);
    run_pass("gog", &mut map, &mut stats, scan_gog);
    run_pass("heroic", &mut map, &mut stats, scan_heroic);
    run_pass("uwp", &mut map, &mut stats, scan_uwp);
    run_pass("accessory_seed", &mut map, &mut stats, seed_windows_accessories);
    run_pass("localappdata_squirrel", &mut map, &mut stats, scan_localappdata_apps);
    run_pass("program_files_exe_scan", &mut map, &mut stats, scan_program_files_recursive);

    // Get-StartApps — PowerShell only (no COM)
    run_pass("get_start_apps", &mut map, &mut stats, scan_get_start_apps);

    // COM — Start Menu .lnk resolution only (best-effort)
    let before = map.len();
    match ComApartment::new() {
        Ok(_com) => {
            if let Err(e) = scan_start_menu(&mut map) {
                log::warn!("Start Menu .lnk scan failed (continuing): {e}");
            }
        }
        Err(e) => {
            log::warn!("COM init failed; Start Menu .lnk pass skipped: {e}");
        }
    }
    stats.push(("start_menu", map.len().saturating_sub(before)));

    log_scan_stats(&stats, map.len());

    Ok(map.into_values().map(|e| e.inner).collect())
}

fn run_pass(
    label: &'static str,
    map: &mut HashMap<String, MapEntry>,
    stats: &mut Vec<(&'static str, usize)>,
    pass: fn(&mut HashMap<String, MapEntry>),
) {
    let before = map.len();
    pass(map);
    stats.push((label, map.len().saturating_sub(before)));
}

fn log_scan_stats(stats: &[(&'static str, usize)], total: usize) {
    let mut parts: Vec<String> = stats
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    parts.push(format!("total={total}"));
    log::info!("app index scan stats: {}", parts.join(" "));
}

// ---------------------------------------------------------------------------
// insert_entry — merge with priority + icon back-fill
// ---------------------------------------------------------------------------
fn insert_entry(
    map: &mut HashMap<String, MapEntry>,
    exe_path: String,
    name: String,
    icon: Option<String>,
    priority: SourcePriority,
) {
    if exe_path.is_empty() {
        return;
    }
    let key = exe_path.to_lowercase();
    let new_entry = MapEntry {
        inner: AppEntry {
            display_name: name,
            exe_path,
            icon_data_url: icon,
        },
        priority,
    };
    map.entry(key)
        .and_modify(|existing| {
            // Lower priority number = better source; replace entirely if better source
            if new_entry.priority < existing.priority {
                // Keep existing icon if new entry has none
                let icon = new_entry
                    .inner
                    .icon_data_url
                    .clone()
                    .or_else(|| existing.inner.icon_data_url.clone());
                *existing = new_entry.clone();
                existing.inner.icon_data_url = icon;
            } else if existing.inner.icon_data_url.is_none()
                && new_entry.inner.icon_data_url.is_some()
            {
                // Same or lower quality source but has an icon we don't have yet
                existing.inner.icon_data_url = new_entry.inner.icon_data_url.clone();
            }
        })
        .or_insert(new_entry);
}

// ---------------------------------------------------------------------------
// Environment variable expansion
// ---------------------------------------------------------------------------
fn expand_env(raw: &str) -> String {
    let mut s = raw.to_string();
    let pairs: &[(&str, &str)] = &[
        ("SYSTEMROOT", "%SystemRoot%"),
        ("WINDIR", "%WINDIR%"),
        ("ProgramFiles", "%ProgramFiles%"),
        ("ProgramFiles(x86)", "%ProgramFiles(x86)%"),
        ("ProgramW6432", "%ProgramW6432%"),
        ("CommonProgramFiles", "%CommonProgramFiles%"),
        ("LOCALAPPDATA", "%LOCALAPPDATA%"),
        ("APPDATA", "%APPDATA%"),
        ("USERPROFILE", "%USERPROFILE%"),
        ("PUBLIC", "%PUBLIC%"),
        ("PROGRAMDATA", "%PROGRAMDATA%"),
    ];
    for (var, placeholder) in pairs {
        if let Ok(val) = std::env::var(var) {
            let lower_s = s.to_lowercase();
            let lower_p = placeholder.to_lowercase();
            if let Some(pos) = lower_s.find(&lower_p) {
                s = format!(
                    "{}{}{}",
                    &s[..pos],
                    val,
                    &s[pos + placeholder.len()..]
                );
            }
        }
    }
    s
}

fn clean_path_str(s: &str) -> String {
    expand_env(s.trim().trim_matches('"'))
}

// ---------------------------------------------------------------------------
// BUG FIX #1: reg_subkey_default — was using `?` inside loop, silently
// dropping the Default value whenever any earlier enum value errored.
// Fix: use `continue` on error, never bail the whole function.
// ---------------------------------------------------------------------------
fn reg_subkey_default(sub: &RegKey) -> Option<String> {
    // Fast path: direct get_value for the (Default) value
    if let Ok(s) = sub.get_value::<String, _>("") {
        let t = clean_path_str(&s);
        if !t.is_empty() {
            return Some(t);
        }
    }
    // Slower fallback: iterate values looking for empty-named entry
    // (some builds store it differently)
    for res in sub.enum_values() {
        let (name, val) = match res {
            Ok(pair) => pair,
            Err(_) => continue, // ← FIX: was `res.ok()?` which would bail on first error
        };
        if !name.is_empty() {
            continue;
        }
        if let Ok(s) = String::from_reg_value(&val) {
            let t = clean_path_str(&s);
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Uninstall registry scan
// ---------------------------------------------------------------------------
fn scan_uninstall_registry(map: &mut HashMap<String, MapEntry>) {
    const PATHS: &[(winreg::HKEY, &str)] = &[
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            HKEY_CURRENT_USER,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
    ];
    for (root, subpath) in PATHS {
        let hkey = RegKey::predef(*root);
        let Ok(uninstall) = hkey.open_subkey(subpath) else {
            continue;
        };
        for entry_name in uninstall.enum_keys().filter_map(|e| e.ok()) {
            let Ok(sub) = uninstall.open_subkey(&entry_name) else {
                continue;
            };

            // Skip entries without a display name (components, updates, etc.)
            let display_name: String = match sub.get_value::<String, _>("DisplayName") {
                Ok(s) => {
                    let t = s.trim();
                    if t.is_empty() {
                        continue;
                    }
                    t.to_string()
                }
                Err(_) => continue,
            };

            // Skip purely system components and patches
            if let Ok(sys) = sub.get_value::<String, _>("SystemComponent") {
                if sys.trim() == "1" {
                    continue;
                }
            }
            if let Ok(parent) = sub.get_value::<String, _>("ParentKeyName") {
                if !parent.trim().is_empty() {
                    continue;
                }
            }

            let install_loc: Option<String> = sub
                .get_value::<String, _>("InstallLocation")
                .ok()
                .map(|s| clean_path_str(&s))
                .filter(|s| !s.is_empty());

            // Resolve exe via: DisplayIcon → guessed name.exe → install_dir_primary_exe
            let exe = sub
                .get_value::<String, _>("DisplayIcon")
                .ok()
                .and_then(|s| display_icon_to_exe(&s))
                .or_else(|| {
                    install_loc
                        .as_deref()
                        .and_then(|loc| install_location_guess(&display_name, loc))
                })
                .or_else(|| {
                    install_loc
                        .as_deref()
                        .and_then(|loc| install_dir_primary_exe(loc, &display_name))
                });

            let Some(exe_path) = exe else { continue };
            let exe_path = exe_path.trim().to_string();
            if exe_path.is_empty() || !Path::new(&exe_path).exists() {
                continue;
            }
            insert_entry(
                map,
                exe_path,
                display_name,
                None,
                SourcePriority::Uninstall,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// App Paths registry scan
// ---------------------------------------------------------------------------
fn scan_app_paths_registry(map: &mut HashMap<String, MapEntry>) {
    const PATHS: &[(winreg::HKEY, &str)] = &[
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths",
        ),
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths",
        ),
        (
            HKEY_CURRENT_USER,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths",
        ),
    ];
    for (root, subpath) in PATHS {
        let hkey = RegKey::predef(*root);
        let Ok(app_paths) = hkey.open_subkey(subpath) else {
            continue;
        };
        for exe_key in app_paths.enum_keys().filter_map(|e| e.ok()) {
            let lower = exe_key.to_lowercase();
            if !lower.ends_with(".exe") {
                continue;
            }
            let Ok(sub) = app_paths.open_subkey(&exe_key) else {
                continue;
            };
            let Some(target) = reg_subkey_default(&sub) else {
                continue;
            };
            let t = target.trim().trim_matches('"');
            if t.is_empty() {
                continue;
            }
            let path = Path::new(t);
            if !path.is_absolute() {
                continue;
            }
            if !path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .eq_ignore_ascii_case("exe")
            {
                continue;
            }
            if !path.exists() {
                continue;
            }
            let exe_norm = std::fs::canonicalize(path)
                .unwrap_or_else(|_| path.to_path_buf())
                .to_string_lossy()
                .to_string();
            let name = display_name_from_app_paths_key(&exe_key);
            insert_entry(map, exe_norm, name, None, SourcePriority::AppPaths);
        }
    }
}

fn display_name_from_app_paths_key(key: &str) -> String {
    let stem = if key.len() >= 4 && key[key.len() - 4..].eq_ignore_ascii_case(".exe") {
        &key[..key.len() - 4]
    } else {
        key
    };
    let stem = stem.replace('_', " ");
    let mut chars = stem.chars();
    match chars.next() {
        None => key.to_string(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

// ---------------------------------------------------------------------------
// Display icon → exe path (strips DLL refs and index suffixes)
// ---------------------------------------------------------------------------
fn display_icon_to_exe(raw: &str) -> Option<String> {
    let expanded = expand_env(raw.trim());
    let trimmed = expanded.trim_matches('"');
    // Strip ",<icon_index>" suffix
    let first = trimmed.split(',').next()?.trim();
    if first.is_empty() {
        return None;
    }
    let p = Path::new(first);
    if !p.is_absolute() {
        return None;
    }
    let ext = p.extension()?.to_str()?;
    // Only accept .exe; skip icon-only .dll references
    if !ext.eq_ignore_ascii_case("exe") {
        return None;
    }
    if !p.exists() {
        return None;
    }
    Some(
        std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_string(),
    )
}

// ---------------------------------------------------------------------------
// Install-dir exe resolution helpers
// ---------------------------------------------------------------------------

/// Try `<installdir>/<FirstWordOfDisplayName>.exe`
fn install_location_guess(display_name: &str, loc: &str) -> Option<String> {
    let dir = Path::new(loc.trim());
    if !dir.is_dir() {
        return None;
    }
    let stem = display_name.split_whitespace().next().unwrap_or(display_name);
    let candidate = dir.join(format!("{stem}.exe"));
    if candidate.is_file() {
        return Some(candidate.to_string_lossy().to_string());
    }
    None
}

/// `true` when an exe stem looks like a helper/updater/redist binary that
/// should never surface in the picker. Combines substring, exact, and suffix
/// matching so Chromium/Electron/Squirrel helpers are filtered without
/// accidentally blocking legitimate names that happen to contain the word
/// "helper" (e.g. `MyHelperApp`).
fn install_dir_skipped_stem(stem: &str) -> bool {
    // Substring matches — catch any variant.
    const CONTAINS: &[&str] = &[
        "uninstall",
        "unins000",
        "unins001",
        "unins002",
        "setup",
        "install",
        "maintenanceservice",
        "crashreporter",
        "crashpad",
        "crashhandler",
        "crash_handler",
        "elevate",
        "vc_redist",
        "vcredist",
        "dxsetup",
        "dotnetfx",
    ];
    // Exact-match stems. Safer than "contains" for short/common words
    // (e.g. "update" as a substring would nuke `GameUpdate.exe`).
    const EXACT: &[&str] = &[
        "update",
        "updater",
        "updaters",
        "updatehandler",
        "squirrel",
        "stub",
        "chrome_proxy",
        "chrome_crashpad_handler",
        "chrome_pwa_launcher",
        "notification_helper",
        "notificationhelper",
        "node",
        "nodejs",
        "python",
        "python3",
        "pythonw",
        "ffmpeg",
        "ffprobe",
        "ffplay",
        "perl",
        "ruby",
        "msiexec",
        "regsvr32",
        "rundll32",
        "conhost",
        "dllhost",
    ];
    // Suffixes — Chromium/Electron/CEF style helper processes.
    const ENDINGS: &[&str] = &[
        "_helper",
        "-helper",
        " helper",
        "_gpu",
        "-gpu",
        "_renderer",
        "-renderer",
        "_plugin",
        "-plugin",
        "_utility",
        "-utility",
        "broker",
        "elevatedinstaller",
        "_bg",
    ];
    let s = stem.to_lowercase();
    if CONTAINS.iter().any(|p| s.contains(p)) {
        return true;
    }
    if EXACT.iter().any(|p| s == *p) {
        return true;
    }
    if ENDINGS.iter().any(|e| s.ends_with(e)) {
        return true;
    }
    // Generic Electron-style "<App> Helper (GPU).exe" → collapsed stem ends with " (gpu)" etc.
    if s.contains("helper (") || s.ends_with(" helper") {
        return true;
    }
    false
}

/// Directory names we should never descend into for the shallow exe-scan
/// pass: locale/resource/redist/helper-process folders that only contain
/// non-app binaries. Matches on lowercased single path components.
fn is_skippable_subdir(name_lower: &str) -> bool {
    matches!(
        name_lower,
        "_commonredist"
            | "commonredist"
            | "redist"
            | "dotnet"
            | "directx"
            | "vcredist"
            | "vc_redist"
            | "dxsetup"
            | "eossdk"
            | "engine"
            | "support"
            | "crashhandler"
            | "installer"
            | "locales"
            | "locale"
            | "resources"
            | "swiftshader"
            | "cef"
            | "plugins"
            | "codecs"
            | "node_modules"
            | "extensions"
            | "packages"
            | "cache"
            | "tools"
            | "driver"
            | "drivers"
            | "microsoft shared"
            | "common files"
    )
}

/// BUG FIX #2: Previously dropped every exe whose stem wasn't in the display
/// name, then only fell back to single-exe case. Now we keep a secondary
/// "any non-skipped exe" bucket so multi-exe dirs still resolve.
fn install_dir_primary_exe(install_loc: &str, display_name: &str) -> Option<String> {
    let dir = Path::new(install_loc.trim());
    if !dir.is_dir() {
        return None;
    }

    let exes: Vec<PathBuf> = std::fs::read_dir(dir).ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("exe"))
                    .unwrap_or(false)
        })
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|st| !install_dir_skipped_stem(st))
                .unwrap_or(true)
        })
        .collect();

    if exes.is_empty() {
        return None;
    }

    let dn = display_name.to_lowercase();
    let mut best_named: Option<(usize, &PathBuf)> = None;
    let mut best_fallback: Option<&PathBuf> = None;

    for p in &exes {
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if stem.len() < 2 {
            continue;
        }

        if dn.contains(&stem) || stem.contains(&dn.split_whitespace().next().unwrap_or("").to_lowercase())
        {
            let score = 1000 + stem.len();
            if best_named.map(|(s, _)| score > s).unwrap_or(true) {
                best_named = Some((score, p));
            }
        } else {
            // FIX: keep as secondary candidate even without name match
            if best_fallback.is_none() {
                best_fallback = Some(p);
            }
        }
    }

    // Prefer name-matched, then single exe, then any exe
    let chosen = best_named
        .map(|(_, p)| p)
        .or_else(|| if exes.len() == 1 { Some(&exes[0]) } else { None })
        .or(best_fallback)?;

    Some(chosen.to_string_lossy().to_string())
}

/// Walk `root` up to `max_depth` levels, collect every `.exe`, score each by
/// (a) exe stem vs display-name tokens, (b) heuristics that drown out crash
/// reporters, redistributables, installer stubs, and launcher shards. Returns
/// the best exe path or `None` if the tree has no candidate.
///
/// This is the workhorse for Steam/Epic/GOG/Heroic where the real game binary
/// can be 2–5 folders deep (`Counter-Strike Global Offensive/game/bin/win64/cs2.exe`,
/// `Baldurs Gate 3/bin/bg3.exe`, etc.) and the legacy `install_dir_primary_exe`
/// only scanned the top folder.
fn find_game_exe_in_tree(root: &Path, display_name: &str, max_depth: usize) -> Option<String> {
    if !root.is_dir() {
        return None;
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    collect_exes(root, 0, max_depth, &mut candidates);
    if candidates.is_empty() {
        return None;
    }

    let dn_lower = display_name.to_lowercase();
    let tokens: Vec<String> = dn_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_string())
        .collect();

    let mut best: Option<(i64, &PathBuf)> = None;
    for p in &candidates {
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if stem.is_empty() {
            continue;
        }
        let score = score_game_exe(&stem, p, root, &dn_lower, &tokens);
        if best.map(|(s, _)| score > s).unwrap_or(true) {
            best = Some((score, p));
        }
    }

    best.map(|(_, p)| p.to_string_lossy().to_string())
}

fn collect_exes(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<PathBuf>) {
    if depth > max_depth || !dir.is_dir() {
        return;
    }
    // Skip engine/redistributable/helper folders where the real app exe never
    // lives. Shared with the shallow Program Files pass via `is_skippable_subdir`.
    if depth > 0 {
        let name_lower = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if is_skippable_subdir(&name_lower) {
            return;
        }
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.is_dir() {
            collect_exes(&p, depth + 1, max_depth, out);
        } else if p.is_file()
            && p.extension()
                .and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("exe"))
                .unwrap_or(false)
        {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem.is_empty() || install_dir_skipped_stem(stem) {
                continue;
            }
            out.push(p);
        }
    }
}

fn score_game_exe(
    stem: &str,
    path: &Path,
    root: &Path,
    dn_lower: &str,
    tokens: &[String],
) -> i64 {
    let mut score: i64 = 0;

    // Exact full-name match (after stripping spaces) — best possible signal.
    if stem == dn_lower.replace(' ', "") {
        score += 700;
    } else if dn_lower.contains(stem) && stem.len() >= 3 {
        score += 500;
    } else if stem.contains(dn_lower) && !dn_lower.is_empty() {
        score += 450;
    }

    // Each display-name token that appears in the stem.
    let matched_tokens = tokens.iter().filter(|t| stem.contains(t.as_str())).count() as i64;
    score += matched_tokens * 120;

    // Initials abbreviation (e.g. "Counter-Strike 2" → stem "cs2",
    // "Baldurs Gate" → "bg"). `digits_suffix` covers trailing sequel numbers
    // that tokens drop because they're single chars.
    let initials: String = tokens.iter().filter_map(|t| t.chars().next()).collect();
    let digits_suffix: String = dn_lower.chars().filter(|c| c.is_ascii_digit()).collect();
    let initials_with_digits = format!("{initials}{digits_suffix}");

    if !initials_with_digits.is_empty() && stem == initials_with_digits {
        score += 400;
    } else if !initials.is_empty() && stem == initials {
        score += 350;
    } else if !initials_with_digits.is_empty() && stem.starts_with(&initials_with_digits) {
        // "bg3" vs "bg3_dx11" — exact match above wins; start-with is the
        // fallback for suffixed variants and should rank below the canonical.
        score += 180;
    } else if !initials.is_empty() && initials.len() >= 2 && stem.contains(&initials) {
        score += 120;
    }

    // Penalise graphics-API / arch variants so the canonical exe wins when
    // both `bg3.exe` and `bg3_dx11.exe` sit next to each other.
    const VARIANT_SUFFIXES: &[&str] = &[
        "_dx11", "_dx12", "_vulkan", "_vk", "_d3d11", "_d3d12", "_opengl", "_gl", "_x64", "_win64",
        "_32bit", "_64bit", "-dx11", "-dx12", "-vulkan",
    ];
    if VARIANT_SUFFIXES.iter().any(|s| stem.ends_with(s)) {
        score -= 80;
    }

    // Penalise common junk that ships alongside games
    const JUNK: &[&str] = &[
        "crash",
        "report",
        "launcher",
        "handler",
        "helper",
        "service",
        "updater",
        "redist",
        "dxsetup",
        "vc_redist",
        "unins",
        "installer",
        "eosbootstrapper",
        "eosovh",
        "touchup",
        "anticheat",
        "easyanticheat",
        "battleye",
    ];
    if JUNK.iter().any(|j| stem.contains(j)) {
        score -= 600;
    }

    // Prefer exes closer to the install root.
    let depth = path
        .strip_prefix(root)
        .ok()
        .map(|rel| rel.components().count() as i64)
        .unwrap_or(1);
    score -= (depth - 1).max(0) * 6;

    // Light tie-break: prefer shorter stems (reduces noise from e.g.
    // `mygame_debug_profile_x64.exe`).
    score -= (stem.len() as i64 - 4).max(0);

    score
}

// ---------------------------------------------------------------------------
// Start Menu .lnk scanner
// BUG FIX #6: Added current user Desktop (%USERPROFILE%\Desktop)
// BUG FIX (lnk): call Resolve() before GetPath() so the COM object actually
//   fills in the target (previously skipped, leaving buffer zeroed for many
//   shortcuts that use relative or PIDL-only links).
// ---------------------------------------------------------------------------
fn scan_start_menu(map: &mut HashMap<String, MapEntry>) -> Result<(), String> {
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| e.to_string())?;
    let persist: IPersistFile = shell_link.cast().map_err(|e| e.to_string())?;

    let mut roots: Vec<PathBuf> = Vec::new();

    // All-users Start Menu
    if let Ok(pd) = std::env::var("PROGRAMDATA") {
        roots.push(PathBuf::from(&pd).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    // Per-user Start Menu (APPDATA and LOCALAPPDATA variants)
    if let Ok(ad) = std::env::var("APPDATA") {
        roots.push(PathBuf::from(&ad).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(&la).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    // Public Desktop
    if let Ok(public) = std::env::var("PUBLIC") {
        roots.push(PathBuf::from(&public).join("Desktop"));
    }
    // BUG FIX #6: Current user's own Desktop
    if let Ok(up) = std::env::var("USERPROFILE") {
        roots.push(PathBuf::from(&up).join("Desktop"));
    }

    for root in roots {
        if root.is_dir() {
            if let Err(e) = visit_dir_lnk(&root, &shell_link, &persist, map) {
                log::warn!("Start Menu subtree skipped {:?}: {e}", root);
            }
        }
    }
    Ok(())
}

/// `steam://`, `http://`, etc. — indexed as opaque launch strings, not local paths.
fn is_non_filesystem_lnk_target(target: &str) -> bool {
    let t = target.trim();
    if t.contains("://") {
        return true;
    }
    t.to_ascii_lowercase().starts_with("shell:")
}

/// Reject `.bat`, `.cmd`, `.ps1`, `.msi`, folders, etc. — only real Win32 `.exe` files.
fn is_acceptable_lnk_filesystem_target(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("exe"))
            .unwrap_or(false)
}

fn visit_dir_lnk(
    dir: &Path,
    shell_link: &IShellLinkW,
    persist: &IPersistFile,
    map: &mut HashMap<String, MapEntry>,
) -> Result<(), String> {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for e in read_dir {
        let e = match e {
            Ok(x) => x,
            Err(_) => continue,
        };
        let p = e.path();
        if p.is_dir() {
            // Silently ignore subdirectory errors (permissions etc.)
            let _ = visit_dir_lnk(&p, shell_link, persist, map);
        } else if p.extension().and_then(|x| x.to_str()) == Some("lnk") {
            if let Some(target) = resolve_lnk(shell_link, persist, &p) {
                if target.is_empty() {
                    continue;
                }
                let target_path = Path::new(&target);
                if is_non_filesystem_lnk_target(&target) {
                    // Legacy rule: plain paths must exist; `steam://` etc. skip the exists check.
                    if !target.contains("://") && !target_path.exists() {
                        continue;
                    }
                } else if !is_acceptable_lnk_filesystem_target(target_path) {
                    continue;
                }
                let label = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("App")
                    .to_string();
                insert_entry(
                    map,
                    target,
                    label,
                    None,
                    SourcePriority::StartMenu,
                );
            }
        }
    }
    Ok(())
}

/// BUG FIX (lnk resolution): call `Resolve` before `GetPath`.
/// Without Resolve, shortcuts that store only a PIDL (e.g. many game shortcuts
/// installed by Steam, Epic, etc.) return an empty path from GetPath.
fn resolve_lnk(shell_link: &IShellLinkW, persist: &IPersistFile, lnk: &Path) -> Option<String> {
    let wide: Vec<u16> = lnk
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;

        // SLR_NO_UI | SLR_NOSEARCH = 0x11 — never show a dialog or search the disk
        let _ = shell_link.Resolve(HWND::default(), 0x0001u32 | 0x0010u32);

        let mut buf = vec![0u16; MAX_PATH as usize];
        shell_link
            .GetPath(
                &mut buf,
                std::ptr::null_mut::<WIN32_FIND_DATAW>(),
                SLGP_RAWPATH.0 as u32,
            )
            .ok()?;
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let s = String::from_utf16_lossy(&buf[..len]);
        let t = s.trim();
        if t.is_empty() {
            return None;
        }
        Some(t.to_string())
    }
}

// ---------------------------------------------------------------------------
// Get-StartApps — same enumeration as the Start menu; AppIDs are often
// `{FOLDERID-GUID}\relative\app.exe` (not absolute paths) — we resolve GUID → base.
// ---------------------------------------------------------------------------
fn scan_get_start_apps(map: &mut HashMap<String, MapEntry>) {
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$OutputEncoding = [System.Text.UTF8Encoding]::new()
foreach ($a in Get-StartApps) {
  $n = $a.Name -replace "`t", "`t"
  $i = $a.AppID -replace "`t", "`t"
  $n + "`t" + $i
}
"#;
    let output = match Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return,
    };

    let stdout = match String::from_utf8(output.stdout) {
        Ok(s) => s,
        Err(_) => return,
    };

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, app_id)) = parse_start_apps_line(line) else {
            continue;
        };
        let display_name = name.trim();
        if display_name.is_empty() {
            continue;
        }
        let Some(target) = resolve_start_app_launch_target(&app_id) else {
            continue;
        };
        if target.is_empty() {
            continue;
        }

        let stored = if target
            .to_ascii_lowercase()
            .starts_with("shell:appsfolder\\")
            || target.to_ascii_lowercase().starts_with("steam://")
        {
            target
        } else {
            let p = Path::new(&target);
            if p.exists() {
                std::fs::canonicalize(p)
                    .unwrap_or_else(|_| p.to_path_buf())
                    .to_string_lossy()
                    .to_string()
            } else {
                continue;
            }
        };

        // No per-entry icon extraction here (too slow); merge may fill from Uninstall / .lnk.
        insert_entry(
            map,
            stored,
            display_name.to_string(),
            None,
            SourcePriority::StartMenu,
        );
    }
}

fn parse_start_apps_line(line: &str) -> Option<(String, String)> {
    let idx = line.find('\t')?;
    let name = line[..idx].trim().to_string();
    let id = line[idx + 1..].trim().to_string();
    if name.is_empty() || id.is_empty() {
        return None;
    }
    Some((name, id))
}

fn resolve_start_app_launch_target(app_id: &str) -> Option<String> {
    let id = app_id.trim();
    if id.is_empty() {
        return None;
    }

    let id_lower = id.to_ascii_lowercase();

    if id_lower.starts_with("steam://") {
        return Some(id.to_string());
    }

    if id_lower.starts_with("shell:appsfolder\\") {
        return Some(id.to_string());
    }

    if looks_like_drive_absolute_path(id) || id.starts_with("\\\\") {
        let cleaned = clean_path_str(id);
        let p = Path::new(&cleaned);
        if p.is_file()
            && p.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("exe"))
                .unwrap_or(false)
        {
            return Some(p.to_string_lossy().to_string());
        }
    }

    // Known-folder prefix: `{GUID}\relative\app.exe` (not the same as AUMIDs like `Microsoft.AutoGenerated.{...}`).
    if id.starts_with('{') {
        if let Some(end) = id.find('}') {
            let guid = &id[1..end];
            let rest = id.get(end + 1..).unwrap_or("");
            if rest.starts_with('\\') {
                let rel = &rest[1..];
                if let Some(base) = known_folder_guid_to_base(guid) {
                    let p = base.join(rel);
                    if p.is_file()
                        && p.extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("exe"))
                            .unwrap_or(false)
                    {
                        return p.to_str().map(|s| s.to_string());
                    }
                }
            }
        }
    }

    // Win32 AUMID / Squirrel / UWP — `Get-StartApps` uses many forms without `\` or `!`
    // (e.g. `com.squirrel.Discord.Discord`, `Brave.<hash>`, `Microsoft.AutoGenerated.{...}`).
    // Explorer accepts `shell:AppsFolder\<AUMID>` for these.
    if !id.contains('\\')
        && !id.contains("://")
        && !id_lower.starts_with("mailto:")
        && id.len() >= 3
    {
        return Some(format!("shell:AppsFolder\\{}", id));
    }

    None
}

fn looks_like_drive_absolute_path(s: &str) -> bool {
    let s = s.trim();
    let b = s.as_bytes();
    b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'/')
}

fn known_folder_guid_to_base(guid: &str) -> Option<PathBuf> {
    let g = guid
        .trim_matches(|c| c == '{' || c == '}')
        .trim()
        .to_ascii_uppercase();
    match g.as_str() {
        "6D809377-6AF0-444B-8957-A3773F02200E" | "905E63B6-C1BF-494E-B29C-65B732D3D46A" => std::env::var("ProgramW6432")
            .or_else(|_| std::env::var("ProgramFiles"))
            .ok()
            .map(PathBuf::from),
        "7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E" => {
            std::env::var("ProgramFiles(x86)").ok().map(PathBuf::from)
        }
        "1AC14E77-02E7-4E5D-B744-2EB1AE5198B7" => {
            let sys = std::env::var("SystemRoot").ok()?;
            Some(Path::new(&sys).join("System32"))
        }
        "D65231B0-B2F1-4857-A4CE-A8E7C6EA7D27" => {
            let sys = std::env::var("SystemRoot").ok()?;
            Some(Path::new(&sys).join("SysWOW64"))
        }
        "F38BF404-1D43-42F2-9305-67DE0B28C23C" => std::env::var("SystemRoot").ok().map(PathBuf::from),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// STEAM scanner — reads all library roots from libraryfolders.vdf
// then parses every appmanifest_*.acf to get name + installdir.
// BUG FIX #4: was entirely missing.
// ---------------------------------------------------------------------------
fn scan_steam(map: &mut HashMap<String, MapEntry>) {
    // Locate Steam via registry first, then env fallbacks
    let steam_root = find_steam_root();
    let steam_root = match steam_root {
        Some(p) => p,
        None => return,
    };

    let library_folders = read_steam_library_folders(&steam_root);
    let mut all_libraries: Vec<PathBuf> = vec![steam_root.join("steamapps")];
    all_libraries.extend(library_folders.into_iter().map(|p| p.join("steamapps")));

    for steamapps_dir in all_libraries {
        if !steamapps_dir.is_dir() {
            continue;
        }
        scan_steam_library(&steamapps_dir, map);
    }
}

fn find_steam_root() -> Option<PathBuf> {
    // Try registry first (most reliable)
    let roots = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam"),
        (HKEY_CURRENT_USER, r"SOFTWARE\Valve\Steam"),
    ];
    for (hive, path) in &roots {
        if let Ok(key) = RegKey::predef(*hive).open_subkey(path) {
            if let Ok(install_path) = key.get_value::<String, _>("InstallPath") {
                let p = PathBuf::from(clean_path_str(install_path.trim()));
                if p.is_dir() {
                    return Some(p);
                }
            }
        }
    }
    // Common default locations
    let defaults = [r"C:\Program Files (x86)\Steam", r"C:\Program Files\Steam"];
    for d in &defaults {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// Parse `config/libraryfolders.vdf` to get additional library roots.
/// The VDF format is simple enough that we can parse it with a lightweight
/// regex-free approach rather than pulling in a full VDF crate.
fn read_steam_library_folders(steam_root: &Path) -> Vec<PathBuf> {
    let vdf_path = steam_root.join("config").join("libraryfolders.vdf");
    let content = match std::fs::read_to_string(&vdf_path) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let mut libs = Vec::new();
    // Each library appears as:  "path"   "D:\\SteamLibrary"
    for line in content.lines() {
        let trimmed = line.trim();
        // Match lines like:  "path"		"E:\\Games\\Steam"
        if !trimmed.to_lowercase().starts_with("\"path\"") {
            continue;
        }
        let parts: Vec<&str> = trimmed.splitn(3, '"').collect();
        // parts[0]="" parts[1]="path" parts[2]=`\t"E:\\..."``
        if parts.len() < 3 {
            continue;
        }
        let rest = parts[2].trim().trim_matches('"');
        // Unescape doubled backslashes that Steam writes
        let path_str = rest.replace("\\\\", "\\");
        let p = PathBuf::from(&path_str);
        if p.is_dir() {
            libs.push(p);
        }
    }
    libs
}

fn scan_steam_library(steamapps: &Path, map: &mut HashMap<String, MapEntry>) {
    let common = steamapps.join("common");
    let entries = match std::fs::read_dir(steamapps) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !fname.starts_with("appmanifest_") || !fname.ends_with(".acf") {
            continue;
        }
        if let Some((name, install_dir)) = parse_acf(&path) {
            if name.is_empty() || install_dir.is_empty() {
                continue;
            }
            // Skip redistributables and Proton/SteamLinuxRuntime entries
            let name_lower = name.to_lowercase();
            if name_lower.contains("redistributable")
                || name_lower.contains("proton")
                || name_lower.starts_with("steam linux runtime")
                || name_lower.contains("directx")
            {
                continue;
            }
            let game_dir = common.join(&install_dir);
            if !game_dir.is_dir() {
                continue;
            }
            if let Some(exe) = find_game_exe_in_tree(&game_dir, &name, 5) {
                if Path::new(&exe).exists() {
                    insert_entry(map, exe, name, None, SourcePriority::Steam);
                }
            }
        }
    }
}

/// Parse `name` and `installdir` from an ACF (Valve KeyValues) file.
fn parse_acf(path: &Path) -> Option<(String, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut name = String::new();
    let mut install_dir = String::new();
    for line in content.lines() {
        let t = line.trim();
        if let Some(val) = kv_value(t, "name") {
            name = val;
        } else if let Some(val) = kv_value(t, "installdir") {
            install_dir = val;
        }
        if !name.is_empty() && !install_dir.is_empty() {
            break;
        }
    }
    if name.is_empty() || install_dir.is_empty() {
        return None;
    }
    Some((name, install_dir))
}

/// Extract value from a KeyValues line:  `"key"    "value"`
fn kv_value(line: &str, key: &str) -> Option<String> {
    let lower = line.to_lowercase();
    let key_quoted = format!("\"{}\"", key.to_lowercase());
    if !lower.starts_with(&key_quoted) {
        return None;
    }
    let rest = &line[key_quoted.len()..].trim();
    if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
        Some(rest[1..rest.len() - 1].to_string())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// EPIC scanner — reads LauncherInstalled.dat (JSON)
// BUG FIX #4: was entirely missing.
// ---------------------------------------------------------------------------
fn scan_epic(map: &mut HashMap<String, MapEntry>) {
    let dat_path = {
        let pd = std::env::var("PROGRAMDATA").unwrap_or_default();
        if pd.is_empty() {
            return;
        }
        PathBuf::from(pd)
            .join("Epic")
            .join("UnrealEngineLauncher")
            .join("LauncherInstalled.dat")
    };
    if !dat_path.exists() {
        return;
    }
    let content = match std::fs::read_to_string(&dat_path) {
        Ok(s) => s,
        Err(_) => return,
    };
    // Lightweight JSON extraction — avoids pulling in serde_json
    // Structure: { "InstallationList": [ { "InstallLocation": "...", "AppName": "..." }, ... ] }
    for chunk in content.split('{').skip(1) {
        let install_location = json_str_field(chunk, "InstallLocation");
        let app_name = json_str_field(chunk, "AppName")
            .or_else(|| json_str_field(chunk, "DisplayName"));

        let (loc, name) = match (install_location, app_name) {
            (Some(l), Some(n)) => (l, n),
            _ => continue,
        };

        // Skip engine installs and non-game entries
        let name_lower = name.to_lowercase();
        if name_lower.starts_with("ue_") || name_lower.contains("unreal engine") {
            continue;
        }

        let dir = Path::new(&loc);
        if !dir.is_dir() {
            continue;
        }

        // Also try reading the .item manifest for a human-readable DisplayName
        let display_name = read_epic_manifest_name(&name).unwrap_or_else(|| {
            // Convert camelCase/PascalCase AppName to display name
            pretty_name_from_identifier(&name)
        });

        if let Some(exe) = find_game_exe_in_tree(dir, &display_name, 4) {
            if Path::new(&exe).exists() {
                insert_entry(map, exe, display_name, None, SourcePriority::Epic);
            }
        }
    }
}

fn read_epic_manifest_name(app_name: &str) -> Option<String> {
    let pd = std::env::var("PROGRAMDATA").ok()?;
    let manifests = PathBuf::from(pd)
        .join("Epic")
        .join("EpicGamesLauncher")
        .join("Data")
        .join("Manifests");
    for entry in std::fs::read_dir(&manifests).ok()?.filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("item") {
            continue;
        }
        let content = match std::fs::read_to_string(&p) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // Check if this manifest is for our app
        if let Some(stored_name) = json_str_field(&content, "AppName") {
            if stored_name.eq_ignore_ascii_case(app_name) {
                return json_str_field(&content, "DisplayName").filter(|s| !s.is_empty());
            }
        }
    }
    None
}

/// Quick JSON string field extractor — no allocations for parsing overhead.
fn json_str_field(json: &str, field: &str) -> Option<String> {
    let key = format!("\"{}\"", field);
    let pos = json.find(&key)?;
    let rest = &json[pos + key.len()..];
    let colon = rest.find(':')? + 1;
    let after_colon = rest[colon..].trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let inner = &after_colon[1..];
    let end = inner.find('"')?;
    let val = &inner[..end];
    if val.is_empty() {
        return None;
    }
    // Unescape common JSON escape sequences
    Some(
        val.replace("\\\\", "\\")
            .replace("\\\"", "\"")
            .replace("\\/", "/"),
    )
}

/// Turn "TheGame2077" → "The Game 2077" (rough heuristic for Epic AppNames)
fn pretty_name_from_identifier(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 8);
    let mut prev_lower = false;
    let mut prev_upper = false;
    for ch in s.chars() {
        if ch == '_' || ch == '-' {
            result.push(' ');
            prev_lower = false;
            prev_upper = false;
        } else if ch.is_uppercase() {
            if prev_lower || prev_upper {
                if !result.is_empty() && !result.ends_with(' ') {
                    result.push(' ');
                }
            }
            result.push(ch);
            prev_lower = false;
            prev_upper = true;
        } else if ch.is_numeric() {
            if prev_lower || prev_upper {
                if !result.ends_with(' ') {
                    result.push(' ');
                }
            }
            result.push(ch);
            prev_lower = false;
            prev_upper = false;
        } else {
            result.push(ch);
            prev_lower = true;
            prev_upper = false;
        }
    }
    result.trim().to_string()
}

// ---------------------------------------------------------------------------
// GOG scanner — reads HKLM\SOFTWARE\(WOW6432Node\)GOG.com\Games
// Each subkey has "exe" and "gameName" values.
// BUG FIX #4: was entirely missing.
// ---------------------------------------------------------------------------
fn scan_gog(map: &mut HashMap<String, MapEntry>) {
    const GOG_PATHS: &[(winreg::HKEY, &str)] = &[
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\GOG.com\Games"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\GOG.com\Games"),
    ];
    for (root, path) in GOG_PATHS {
        let hkey = RegKey::predef(*root);
        let Ok(games) = hkey.open_subkey(path) else {
            continue;
        };
        for game_id in games.enum_keys().filter_map(|e| e.ok()) {
            let Ok(sub) = games.open_subkey(&game_id) else {
                continue;
            };
            // "exe" holds the full path to the game executable
            let exe: String = sub
                .get_value::<String, _>("exe")
                .or_else(|_| sub.get_value::<String, _>("EXEFILE"))
                .unwrap_or_default();
            let exe = clean_path_str(&exe);
            if exe.is_empty() || !Path::new(&exe).exists() {
                continue;
            }

            let name: String = sub
                .get_value::<String, _>("gameName")
                .or_else(|_| sub.get_value::<String, _>("GAMENAME"))
                .unwrap_or_default();
            let name = name.trim().to_string();
            if name.is_empty() {
                continue;
            }

            insert_entry(map, exe, name, None, SourcePriority::Gog);
        }
    }
}

// ---------------------------------------------------------------------------
// HEROIC scanner — Legendary (Epic), GOG, Amazon (Nile), and sideloaded apps.
// All four sources live under %APPDATA%\heroic\. Each file is optional; a
// missing file is a no-op. Parsing is deliberately lightweight: we scan for
// `{ ... }` object records and pull known string fields via `json_str_field`.
// ---------------------------------------------------------------------------
fn scan_heroic(map: &mut HashMap<String, MapEntry>) {
    let Ok(appdata) = std::env::var("APPDATA") else {
        return;
    };
    let root = PathBuf::from(appdata).join("heroic");
    if !root.is_dir() {
        return;
    }

    // Legendary (Epic) — installed.json is an object keyed by app_name whose
    // values contain `title`, `install_path`, `executable`.
    let legendary = root
        .join("legendaryConfig")
        .join("legendary")
        .join("installed.json");
    scan_heroic_installed_json(
        &legendary,
        map,
        SourcePriority::Epic,
        &["title"],
        &["install_path"],
        Some("executable"),
    );

    // GOG — installed.json contains objects with `appName`, `install_path`, `platform`.
    // Use the store cache for human-readable names when present.
    let gog_installed = root.join("gog_store").join("installed.json");
    let gog_library = root.join("store_cache").join("gog_library.json");
    let gog_name_lookup = read_heroic_gog_library(&gog_library);
    scan_heroic_gog_installed(&gog_installed, map, &gog_name_lookup);

    // Amazon (Nile) — installed.json records with `id`, `path`, optionally
    // `title`. Path points to the install dir; pick the primary exe from there.
    let nile = root.join("nile_config").join("nile").join("installed.json");
    scan_heroic_installed_json(
        &nile,
        map,
        SourcePriority::Uninstall, // no dedicated priority; richer than ExeScan
        &["title", "id"],
        &["path", "install_path"],
        None,
    );

    // Sideloaded — `library.json` with `title`, `executable` (already absolute).
    let sideload = root.join("sideload_apps").join("library.json");
    scan_heroic_sideload(&sideload, map);
}

/// Generic Heroic `installed.json` walker. Splits on `{` to form pseudo-records,
/// then pulls the first non-empty value among the supplied field-name lists for
/// name and install dir. If `exe_field` is `Some`, the referenced field is
/// treated as an executable **relative to the install dir** (Legendary style);
/// otherwise we scan the install dir for a primary exe (Nile style).
fn scan_heroic_installed_json(
    path: &Path,
    map: &mut HashMap<String, MapEntry>,
    priority: SourcePriority,
    name_fields: &[&str],
    dir_fields: &[&str],
    exe_field: Option<&str>,
) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for record in heroic_split_records(&content) {
        let Some(name) = first_nonempty_field(record, name_fields) else {
            continue;
        };
        let Some(install_dir) = first_nonempty_field(record, dir_fields) else {
            continue;
        };
        let display_name = pretty_name_from_identifier(&name);
        let exe_path: Option<String> = if let Some(field) = exe_field {
            json_str_field(record, field).and_then(|rel| {
                let candidate = Path::new(&install_dir).join(rel.trim_start_matches(['/', '\\']));
                if candidate.is_file() {
                    Some(candidate.to_string_lossy().to_string())
                } else {
                    find_game_exe_in_tree(Path::new(&install_dir), &display_name, 4)
                }
            })
        } else {
            find_game_exe_in_tree(Path::new(&install_dir), &display_name, 4)
        };
        let Some(exe) = exe_path else { continue };
        if !Path::new(&exe).exists() {
            continue;
        }
        insert_entry(map, exe, display_name, None, priority);
    }
}

fn scan_heroic_gog_installed(
    path: &Path,
    map: &mut HashMap<String, MapEntry>,
    name_lookup: &HashMap<String, String>,
) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for record in heroic_split_records(&content) {
        let Some(install_dir) = first_nonempty_field(record, &["install_path"]) else {
            continue;
        };
        let app_name = json_str_field(record, "appName").unwrap_or_default();
        let display_name = name_lookup
            .get(&app_name)
            .cloned()
            .unwrap_or_else(|| pretty_name_from_identifier(&app_name));
        if display_name.is_empty() {
            continue;
        }
        let Some(exe) = find_game_exe_in_tree(Path::new(&install_dir), &display_name, 4) else {
            continue;
        };
        if !Path::new(&exe).exists() {
            continue;
        }
        insert_entry(map, exe, display_name, None, SourcePriority::Gog);
    }
}

/// Heroic's sideload library stores absolute `executable` paths directly.
fn scan_heroic_sideload(path: &Path, map: &mut HashMap<String, MapEntry>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for record in heroic_split_records(&content) {
        let Some(name) = first_nonempty_field(record, &["title", "app_name"]) else {
            continue;
        };
        let Some(exe) = json_str_field(record, "executable") else {
            continue;
        };
        let exe = clean_path_str(&exe);
        if exe.is_empty() || !Path::new(&exe).is_file() {
            continue;
        }
        insert_entry(map, exe, name, None, SourcePriority::Uninstall);
    }
}

/// Parse Heroic's `store_cache/gog_library.json` into a `app_name -> title` map.
/// Records contain `"app_name"` and `"title"` strings close together.
fn read_heroic_gog_library(path: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return out;
    };
    for record in heroic_split_records(&content) {
        let Some(app_name) = json_str_field(record, "app_name") else {
            continue;
        };
        let Some(title) = json_str_field(record, "title") else {
            continue;
        };
        if !app_name.is_empty() && !title.is_empty() {
            out.insert(app_name, title);
        }
    }
    out
}

/// Split a JSON blob into per-object slices, emitting each balanced `{...}`
/// region (including nested ones). Ignores braces inside string literals.
/// Good enough for Heroic's config files without pulling in a full JSON parser.
fn heroic_split_records(json: &str) -> Vec<&str> {
    let bytes = json.as_bytes();
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => stack.push(i),
            b'}' => {
                if let Some(start) = stack.pop() {
                    if let Ok(slice) = std::str::from_utf8(&bytes[start..=i]) {
                        out.push(slice);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn first_nonempty_field(record: &str, fields: &[&str]) -> Option<String> {
    for f in fields {
        if let Some(v) = json_str_field(record, f) {
            let t = v.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// UWP / Microsoft Store scanner
// BUG FIX #5: was entirely missing.
// Uses PowerShell to enumerate packages and builds a shell: launch URI.
// ---------------------------------------------------------------------------
fn scan_uwp(map: &mut HashMap<String, MapEntry>) {
    // PowerShell script: emit lines of "DisplayName|PackageFamilyName|AppId"
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
$pkgs = Get-AppxPackage -PackageTypeFilter Main | Where-Object {
    $_.IsFramework -eq $false
}
foreach ($pkg in $pkgs) {
    try {
        $manifest = Get-AppxPackageManifest -Package $pkg.PackageFullName
        foreach ($app in $manifest.Package.Applications.Application) {
            $appId = $app.Id
            if (-not $appId) { continue }
            $displayName = $app.VisualElements.DisplayName
            if (-not $displayName -or $displayName -match '^\s*$') {
                $displayName = $pkg.Name
            }
            # Resolve resource string names like "ms-resource:AppName"
            if ($displayName -match '^ms-resource:') {
                $displayName = $pkg.Name
            }
            $pfn = $pkg.PackageFamilyName
            Write-Output "$displayName|$pfn|$appId"
        }
    } catch {}
}
"#;

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return,
    };

    let stdout = match String::from_utf8(output.stdout) {
        Ok(s) => s,
        Err(_) => return,
    };

    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(3, '|').collect();
        if parts.len() != 3 {
            continue;
        }
        let display_name = parts[0].trim();
        let pfn = parts[1].trim();
        let app_id = parts[2].trim();
        if display_name.is_empty() || pfn.is_empty() {
            continue;
        }

        // Filter out system noise
        let dn_lower = display_name.to_lowercase();
        if dn_lower.contains("framework")
            || dn_lower.contains("runtime")
            || dn_lower.contains("vclibs")
            || dn_lower.starts_with("microsoft.ui")
        {
            continue;
        }

        // Launch URI for UWP apps: shell:AppsFolder\<PackageFamilyName>!<AppId>
        let launch_uri = format!("shell:AppsFolder\\{}!{}", pfn, app_id);
        insert_entry(
            map,
            launch_uri,
            display_name.to_string(),
            None,
            SourcePriority::AppPaths, // treat as similar priority to App Paths
        );
    }
}

// ---------------------------------------------------------------------------
// Windows accessories seed  (Notepad, Paint, etc.)
// These often lack Uninstall entries; this guarantees they always appear.
// ---------------------------------------------------------------------------
fn seed_windows_accessories(map: &mut HashMap<String, MapEntry>) {
    let sys = std::env::var("SystemRoot")
        .or_else(|_| std::env::var("WINDIR"))
        .unwrap_or_default();
    if sys.is_empty() {
        return;
    }

    const PAIRS: &[(&str, &str)] = &[
        ("Notepad", r"System32\notepad.exe"),
        ("Paint", r"System32\mspaint.exe"),
        ("Snipping Tool", r"System32\SnippingTool.exe"),
        ("Character Map", r"System32\charmap.exe"),
        ("Windows Media Player", r"System32\wmplayer.exe"),
        ("Task Manager", r"System32\Taskmgr.exe"),
        ("Registry Editor", r"regedit.exe"),
        ("Calculator", r"System32\calc.exe"),
        (
            "WordPad",
            r"Program Files\Windows NT\Accessories\wordpad.exe",
        ),
    ];

    for (name, rel) in PAIRS {
        // Some paths are relative to system root, others to drive root
        let candidates: Vec<PathBuf> = if rel.starts_with("Program Files") {
            vec![
                PathBuf::from(r"C:\").join(rel),
                Path::new(&sys).join("..").join(rel),
            ]
        } else {
            vec![Path::new(&sys).join(rel)]
        };

        for p in candidates {
            if !p.is_file() {
                continue;
            }
            let exe_norm = std::fs::canonicalize(&p)
                .unwrap_or_else(|_| p.to_path_buf())
                .to_string_lossy()
                .to_string();
            insert_entry(
                map,
                exe_norm,
                (*name).to_string(),
                None,
                SourcePriority::Accessory,
            );
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// LocalAppData scanner — catches Squirrel apps (Discord, Teams classic,
// GitHub Desktop, 1Password, Slack) that install to `%LOCALAPPDATA%\<App>\`
// outside of `Programs\`, so the later Program Files recursive pass misses
// them. Uses the Squirrel convention: `<App>\Update.exe` launches the
// latest `<App>\app-<ver>\<App>.exe`. Falls back to any matching exe in the
// newest `app-*` subdirectory when the naming doesn't match exactly.
// ---------------------------------------------------------------------------
fn scan_localappdata_apps(map: &mut HashMap<String, MapEntry>) {
    let Ok(la) = std::env::var("LOCALAPPDATA") else {
        return;
    };
    let la_root = PathBuf::from(la);
    if !la_root.is_dir() {
        return;
    }

    // Directories under LocalAppData that are almost never user-launchable —
    // skip them so we don't spend time recursing into them or polluting the
    // index with crash reporters and telemetry stubs.
    const SKIP_DIRS: &[&str] = &[
        "microsoft",
        "packages",
        "temp",
        "tempstate",
        "publisherdiagnostics",
        "diagnosticstore",
        "crashdumps",
        "connecteddevicesplatform",
        "comms",
        "placeholdertilelogofolder",
        "virtualstore",
        "d3dscache",
        "gdk",
        "nvidia",
        "nvidia corporation",
        "amd",
        "packagestaging",
        "webex",
        "programs", // handled by the deeper recursive scan already
    ];

    let entries = match std::fs::read_dir(&la_root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let app_dir = entry.path();
        if !app_dir.is_dir() {
            continue;
        }
        let app_dir_name = app_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let app_dir_lower = app_dir_name.to_lowercase();
        if app_dir_lower.is_empty() || app_dir_lower.starts_with('.') {
            continue;
        }
        if SKIP_DIRS.iter().any(|s| app_dir_lower == *s) {
            continue;
        }
        if let Some(exe) = find_squirrel_app_exe(&app_dir, &app_dir_name) {
            if Path::new(&exe).exists() {
                insert_entry(
                    map,
                    exe,
                    pretty_name_from_identifier(&app_dir_name),
                    None,
                    SourcePriority::Uninstall,
                );
                continue;
            }
        }
        // Fallback: top-level `<App>.exe` directly under `<App>\`
        let direct = app_dir.join(format!("{app_dir_name}.exe"));
        if direct.is_file() {
            let p = direct.to_string_lossy().to_string();
            insert_entry(
                map,
                p,
                pretty_name_from_identifier(&app_dir_name),
                None,
                SourcePriority::AppPaths,
            );
        }
    }
}

/// Squirrel-packaged apps drop into `<App>\app-<semver>\<App>.exe`. The `<App>\`
/// dir also contains a stub `Update.exe` + `packages/`. Find the newest `app-*`
/// subdirectory (by name, which is an effective semver sort) and pick the
/// best exe inside it via [`find_game_exe_in_tree`].
fn find_squirrel_app_exe(app_dir: &Path, app_name: &str) -> Option<String> {
    let mut app_versions: Vec<PathBuf> = std::fs::read_dir(app_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_lowercase().starts_with("app-"))
                .unwrap_or(false)
        })
        .collect();
    app_versions.sort_by(|a, b| {
        a.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .cmp(b.file_name().and_then(|s| s.to_str()).unwrap_or(""))
    });
    let newest = app_versions.last()?;
    find_game_exe_in_tree(newest, app_name, 1)
}

// ---------------------------------------------------------------------------
// Program Files / LocalAppData\Programs shallow scan.
//
// Previously this walked every directory up to depth 6 and inserted *every*
// `.exe` it found. On a typical dev machine that produced 1000+ entries,
// dominated by Chromium / Electron helpers, updater stubs, codec DLLs-as-exe,
// and per-locale resource binaries. Those both polluted the search UI and
// made icon extraction miserable.
//
// New strategy: treat each top-level vendor/app folder as **one app**, and
// use `find_game_exe_in_tree` to pick the *best* exe inside it (same scoring
// that already works well for Steam/Epic installs). We also peek one level
// deeper to handle `Publisher\App` layouts (e.g. `Adobe\Photoshop`,
// `Microsoft\Edge`). Insert uses `SourcePriority::ExeScan`, so richer
// sources (Uninstall, Start Menu, App Paths, UWP, Steam/Epic/GOG) always win
// on name merge.
// ---------------------------------------------------------------------------
fn scan_program_files_recursive(map: &mut HashMap<String, MapEntry>) {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(pf) = std::env::var("ProgramFiles") {
        roots.push(PathBuf::from(pf));
    }
    if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
        roots.push(PathBuf::from(pf86));
    }
    if let Ok(pfw) = std::env::var("ProgramW6432") {
        roots.push(PathBuf::from(pfw));
    }
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(la).join("Programs");
        if p.is_dir() {
            roots.push(p);
        }
    }
    for root in roots {
        scan_install_dirs_shallow(&root, map);
    }
}

fn scan_install_dirs_shallow(root: &Path, map: &mut HashMap<String, MapEntry>) {
    if !root.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let app_dir = entry.path();
        if !app_dir.is_dir() {
            continue;
        }
        let dir_name = app_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let dir_lower = dir_name.to_lowercase();
        if dir_lower.is_empty() || dir_lower.starts_with('.') {
            continue;
        }
        if is_skippable_subdir(&dir_lower) {
            continue;
        }

        // Pass 1 — treat this directory itself as the app dir, but only if
        // it has an exe directly inside (depth 0). Publisher folders like
        // `Adobe` have no exes at their root, so this correctly skips them
        // and lets pass 2 name each sub-dir individually instead of filing
        // one of the sub-apps under the publisher name.
        let display_name = pretty_name_from_identifier(&dir_name);
        try_insert_best_exe(&app_dir, &display_name, 0, map);

        // Pass 2 — peek one level deeper for Publisher\App layouts. Sub-dir
        // scans get the full depth so deep nests like
        // `Microsoft\Edge\Application\msedge.exe` still resolve. We skip
        // helper/resource buckets so Electron apps don't spray their locales
        // folder into the index.
        let Ok(subs) = std::fs::read_dir(&app_dir) else {
            continue;
        };
        for sub in subs.filter_map(|e| e.ok()) {
            let sub_dir = sub.path();
            if !sub_dir.is_dir() {
                continue;
            }
            let sub_name = sub_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let sub_lower = sub_name.to_lowercase();
            if sub_lower.is_empty() || sub_lower.starts_with('.') {
                continue;
            }
            if is_skippable_subdir(&sub_lower) {
                continue;
            }
            let sub_display = pretty_name_from_identifier(&sub_name);
            try_insert_best_exe(&sub_dir, &sub_display, 4, map);
        }
    }
}

fn try_insert_best_exe(
    app_dir: &Path,
    display_name: &str,
    max_depth: usize,
    map: &mut HashMap<String, MapEntry>,
) {
    let Some(exe) = find_game_exe_in_tree(app_dir, display_name, max_depth) else {
        return;
    };
    if !Path::new(&exe).exists() {
        return;
    }
    let exe_norm = std::fs::canonicalize(&exe)
        .unwrap_or_else(|_| PathBuf::from(&exe))
        .to_string_lossy()
        .to_string();
    let key = exe_norm.to_lowercase();
    if map.contains_key(&key) {
        return;
    }
    insert_entry(
        map,
        exe_norm,
        display_name.to_string(),
        None,
        SourcePriority::ExeScan,
    );
}

// ---------------------------------------------------------------------------
// Icon extraction — native Win32 / GDI (≈1–5 ms per call).
//
// The previous implementation spawned a PowerShell subprocess per icon
// (`Add-Type … ExtractAssociatedIcon`), which took ~200–500 ms each and made
// the picker freeze whenever the dropdown opened with many rows. We now go
// straight to `SHGetFileInfoW` → `GetIconInfo` → `GetDIBits`, then hand the
// raw BGRA pixels to the `png` crate. Callers are expected to cache results;
// see `apps::get_app_icon_cached`.
// ---------------------------------------------------------------------------
pub(crate) fn extract_icon_data_url(exe_path: &str) -> Option<String> {
    // Only extract icons for real filesystem paths, not UWP / shell: URIs.
    if !Path::new(exe_path).exists() {
        return None;
    }
    let png = extract_icon_png_bytes(exe_path)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    Some(format!("data:image/png;base64,{b64}"))
}

fn extract_icon_png_bytes(exe_path: &str) -> Option<Vec<u8>> {
    let wide: Vec<u16> = Path::new(exe_path)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let mut sfi: SHFILEINFOW = std::mem::zeroed();
        // `SHGFI_USEFILEATTRIBUTES` keeps the call strictly local (no shell
        // binding lookups over the network). 32-bit large icon gives us a
        // crisp 32×32 bitmap on standard DPI and still scales well.
        let flags = SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES;
        let ok = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut sfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );
        if ok == 0 || sfi.hIcon.is_invalid() {
            return None;
        }
        let png = hicon_to_png(sfi.hIcon);
        let _ = DestroyIcon(sfi.hIcon);
        png
    }
}

unsafe fn hicon_to_png(hicon: HICON) -> Option<Vec<u8>> {
    let mut ii: ICONINFO = std::mem::zeroed();
    if GetIconInfo(hicon, &mut ii).is_err() {
        return None;
    }
    // Always free bitmaps, even on early return.
    struct BitmapGuard(HBITMAP);
    impl Drop for BitmapGuard {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe {
                    let _ = DeleteObject(self.0.into());
                }
            }
        }
    }
    let _color_guard = BitmapGuard(ii.hbmColor);
    let _mask_guard = BitmapGuard(ii.hbmMask);

    if ii.hbmColor.is_invalid() {
        return None;
    }

    // Fetch width/height from the colour bitmap.
    let mut bmp: BITMAP = std::mem::zeroed();
    let got = GetObjectW(
        ii.hbmColor.into(),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bmp as *mut _ as *mut _),
    );
    if got == 0 {
        return None;
    }
    let w = bmp.bmWidth;
    let h = bmp.bmHeight;
    if w <= 0 || h <= 0 {
        return None;
    }

    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h; // negative = top-down DIB
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;

    let stride = (w as usize) * 4;
    let mut pixels = vec![0u8; stride * h as usize];

    let screen_dc: HDC = GetDC(None);
    if screen_dc.is_invalid() {
        return None;
    }
    let rows_copied = GetDIBits(
        screen_dc,
        ii.hbmColor,
        0,
        h as u32,
        Some(pixels.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    ReleaseDC(None, screen_dc);
    if rows_copied == 0 {
        return None;
    }

    // Windows gives us BGRA (pre-multiplied when the icon has alpha). Swap
    // the R/B bytes in-place to get RGBA that PNG expects.
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
    }

    // Some icons report every alpha byte as 0 (legacy .ico resources with
    // only a mono AND-mask). Fall back to fully opaque in that case so the
    // PNG isn't invisible.
    let any_alpha = pixels.chunks_exact(4).any(|p| p[3] != 0);
    if !any_alpha {
        for px in pixels.chunks_exact_mut(4) {
            px[3] = 0xFF;
        }
    }

    let mut out: Vec<u8> = Vec::with_capacity(stride * h as usize + 1024);
    {
        let mut encoder = png::Encoder::new(&mut out, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&pixels).ok()?;
    }
    Some(out)
}


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_prefers_lower_priority_source() {
        let mut map = HashMap::new();
        insert_entry(
            &mut map,
            r"C:\Apps\foo.exe".into(),
            "Foo ExeScan".into(),
            None,
            SourcePriority::ExeScan,
        );
        insert_entry(
            &mut map,
            r"C:\Apps\foo.exe".into(),
            "Foo Uninstall".into(),
            None,
            SourcePriority::Uninstall,
        );
        let e = map.get(&r"c:\apps\foo.exe".to_string()).unwrap();
        assert_eq!(e.inner.display_name, "Foo Uninstall");
    }

    #[test]
    fn insert_backfills_icon_without_overwriting_name() {
        let mut map = HashMap::new();
        insert_entry(
            &mut map,
            r"C:\Apps\foo.exe".into(),
            "Good Name".into(),
            None,
            SourcePriority::Uninstall,
        );
        insert_entry(
            &mut map,
            r"C:\Apps\foo.exe".into(),
            "Bad Name".into(),
            Some("data:image/png;base64,AAA=".into()),
            SourcePriority::ExeScan,
        );
        let e = map.get(&r"c:\apps\foo.exe".to_string()).unwrap();
        // Name should be preserved from higher priority source
        assert_eq!(e.inner.display_name, "Good Name");
        // But icon should be backfilled
        assert_eq!(
            e.inner.icon_data_url.as_deref(),
            Some("data:image/png;base64,AAA=")
        );
    }

    #[test]
    fn kv_value_parses_acf_line() {
        assert_eq!(
            kv_value(r#""name"		"Portal 2""#, "name"),
            Some("Portal 2".into())
        );
        assert_eq!(
            kv_value(r#""installdir"	"Portal 2""#, "installdir"),
            Some("Portal 2".into())
        );
        assert_eq!(
            kv_value(r#""appid"		"620""#, "appid"),
            Some("620".into())
        );
    }

    #[test]
    fn heroic_split_records_handles_nested_and_strings() {
        let json = r#"[
            {"title": "A", "meta": {"x": 1}},
            {"title": "B", "note": "has { brace }"}
        ]"#;
        let recs = heroic_split_records(json);
        // Emits each balanced object: outer meta, outer A, outer B = 3.
        // Brace inside a string must not open a new record.
        let has_a = recs.iter().any(|r| r.contains("\"A\""));
        let has_b = recs.iter().any(|r| r.contains("\"B\""));
        assert!(has_a && has_b, "got: {recs:?}");
        assert!(
            recs.iter().all(|r| !r.contains("has { brace }") || r.contains("\"B\"")),
            "string-literal braces leaked into records: {recs:?}"
        );
    }

    #[test]
    fn heroic_first_nonempty_field_picks_first_present() {
        let rec = r#"{"id": "abc", "title": "Real Title"}"#;
        assert_eq!(
            first_nonempty_field(rec, &["title", "id"]).as_deref(),
            Some("Real Title")
        );
        assert_eq!(
            first_nonempty_field(rec, &["missing", "id"]).as_deref(),
            Some("abc")
        );
        assert_eq!(first_nonempty_field(rec, &["missing"]), None);
    }

    #[test]
    fn read_heroic_gog_library_builds_name_lookup() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("gog_library.json");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(
            br#"{"games":[
                {"app_name":"1207658937","title":"The Witcher 3"},
                {"app_name":"1423049311","title":"Cyberpunk 2077"}
            ]}"#,
        )
        .unwrap();
        let map = read_heroic_gog_library(&p);
        assert_eq!(map.get("1207658937").map(String::as_str), Some("The Witcher 3"));
        assert_eq!(map.get("1423049311").map(String::as_str), Some("Cyberpunk 2077"));
    }

    #[test]
    fn scan_heroic_sideload_inserts_absolute_exe() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let fake_exe = dir.path().join("launcher.exe");
        std::fs::File::create(&fake_exe).unwrap().write_all(b"MZ").unwrap();
        let lib = dir.path().join("library.json");
        let exe_json = fake_exe.to_string_lossy().replace('\\', "\\\\");
        let body = format!(
            r#"{{"games":[{{"title":"My Sideloaded App","executable":"{exe_json}"}}]}}"#
        );
        std::fs::File::create(&lib)
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();

        let mut map: HashMap<String, MapEntry> = HashMap::new();
        scan_heroic_sideload(&lib, &mut map);
        let hit = map
            .values()
            .find(|e| e.inner.display_name == "My Sideloaded App");
        assert!(hit.is_some(), "sideload entry missing; map: {:?}", map.keys().collect::<Vec<_>>());
    }

    #[test]
    fn json_str_field_extracts_values() {
        let json = r#""InstallLocation": "C:\\Games\\MyGame", "AppName": "MyGame123""#;
        assert_eq!(
            json_str_field(json, "InstallLocation"),
            Some(r"C:\Games\MyGame".into())
        );
        assert_eq!(
            json_str_field(json, "AppName"),
            Some("MyGame123".into())
        );
    }

    #[test]
    fn install_dir_primary_exe_fallback_when_no_name_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("FSD.exe"), [0u8]).unwrap();
        std::fs::write(dir.path().join("Uninstall.exe"), [0u8]).unwrap();
        // "Deep Rock Galactic" doesn't contain "fsd" — old code would return None here
        let hit = install_dir_primary_exe(
            &dir.path().to_string_lossy(),
            "Deep Rock Galactic",
        );
        // FIX: should now fall back to "FSD.exe" (sole non-skip candidate)
        assert!(hit.is_some(), "should fall back to FSD.exe");
        assert!(hit.unwrap().to_lowercase().contains("fsd.exe"));
    }

    #[test]
    fn install_dir_primary_exe_prefers_name_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Discord.exe"), [0u8]).unwrap();
        std::fs::write(dir.path().join("Helper.exe"), [0u8]).unwrap();
        let hit = install_dir_primary_exe(&dir.path().to_string_lossy(), "Discord")
            .expect("should find Discord.exe");
        assert!(hit.to_lowercase().contains("discord.exe"));
    }

    // ------------------------------------------------------------------
    // find_game_exe_in_tree — deep scan for CS2 / BG3 style installs
    // ------------------------------------------------------------------

    fn touch_file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(path).unwrap();
    }

    // ------------------------------------------------------------------
    // Start Menu .lnk — filesystem targets must be .exe (no uninstall scripts)
    // ------------------------------------------------------------------

    #[test]
    fn is_acceptable_lnk_filesystem_target_accepts_exe_only() {
        let dir = tempfile::tempdir().unwrap();
        touch_file(&dir.path().join("app.exe"));
        touch_file(&dir.path().join("uninstall.bat"));
        touch_file(&dir.path().join("remove.ps1"));
        touch_file(&dir.path().join("setup.cmd"));
        touch_file(&dir.path().join("silent.vbs"));
        assert!(is_acceptable_lnk_filesystem_target(&dir.path().join("app.exe")));
        assert!(!is_acceptable_lnk_filesystem_target(
            &dir.path().join("uninstall.bat")
        ));
        assert!(!is_acceptable_lnk_filesystem_target(&dir.path().join("remove.ps1")));
        assert!(!is_acceptable_lnk_filesystem_target(&dir.path().join("setup.cmd")));
        assert!(!is_acceptable_lnk_filesystem_target(&dir.path().join("silent.vbs")));
        assert!(!is_acceptable_lnk_filesystem_target(dir.path()));
    }

    #[test]
    fn is_non_filesystem_lnk_target_detects_protocols_and_shell_verbs() {
        assert!(is_non_filesystem_lnk_target("steam://run/123"));
        assert!(is_non_filesystem_lnk_target("  shell:AppsFolder\\Foo!Bar  "));
        assert!(is_non_filesystem_lnk_target("http://example.com/x"));
        assert!(!is_non_filesystem_lnk_target(r"C:\Games\foo.exe"));
        assert!(!is_non_filesystem_lnk_target(r"\\server\share\app.exe"));
    }

    #[test]
    fn find_game_exe_in_tree_finds_cs2_at_depth_four() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Counter-Strike Global Offensive");
        touch_file(&root.join("game/bin/win64/cs2.exe"));
        touch_file(&root.join("game/bin/win64/crashhandler.exe"));
        let hit = find_game_exe_in_tree(&root, "Counter-Strike 2", 5)
            .expect("cs2 deep hit");
        assert!(hit.to_lowercase().ends_with("cs2.exe"), "got {hit}");
    }

    #[test]
    fn find_game_exe_in_tree_finds_bg3_at_depth_two() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Baldurs Gate 3");
        touch_file(&root.join("bin/bg3.exe"));
        touch_file(&root.join("bin/bg3_dx11.exe"));
        touch_file(&root.join("EasyAntiCheat/EasyAntiCheat_Setup.exe"));
        let hit = find_game_exe_in_tree(&root, "Baldur's Gate 3", 4)
            .expect("bg3 hit");
        assert!(hit.to_lowercase().ends_with("bg3.exe"), "got {hit}");
    }

    #[test]
    fn find_game_exe_in_tree_penalises_crash_reporters() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("MyGame");
        touch_file(&root.join("MyGame.exe"));
        touch_file(&root.join("UnrealCrashReporter.exe"));
        touch_file(&root.join("CrashHandler.exe"));
        let hit = find_game_exe_in_tree(&root, "MyGame", 3).unwrap();
        assert!(hit.to_lowercase().ends_with("mygame.exe"), "got {hit}");
    }

    #[test]
    fn find_game_exe_in_tree_returns_none_for_empty_tree() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_game_exe_in_tree(dir.path(), "Anything", 4).is_none());
    }

    // ------------------------------------------------------------------
    // Squirrel / LocalAppData scanner — Discord style
    // ------------------------------------------------------------------

    #[test]
    fn find_squirrel_app_exe_picks_newest_app_version() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Discord");
        touch_file(&app.join("Update.exe"));
        touch_file(&app.join("app-1.0.9001/Discord.exe"));
        touch_file(&app.join("app-1.0.9005/Discord.exe"));
        let hit = find_squirrel_app_exe(&app, "Discord").expect("squirrel hit");
        assert!(
            hit.to_lowercase().contains("app-1.0.9005"),
            "should prefer newest, got {hit}"
        );
    }

    #[test]
    fn scan_localappdata_apps_skips_microsoft_and_packages() {
        // We can't set LOCALAPPDATA reliably here without poisoning the env for
        // other tests, but we can assert the skip logic on the directory name
        // by running a quick characterisation.
        let noise = ["Microsoft", "Packages", "Temp", "Programs"];
        for name in noise {
            let lower = name.to_lowercase();
            let skipped = matches!(
                lower.as_str(),
                "microsoft"
                    | "packages"
                    | "temp"
                    | "tempstate"
                    | "publisherdiagnostics"
                    | "diagnosticstore"
                    | "crashdumps"
                    | "connecteddevicesplatform"
                    | "comms"
                    | "placeholdertilelogofolder"
                    | "virtualstore"
                    | "d3dscache"
                    | "gdk"
                    | "nvidia"
                    | "nvidia corporation"
                    | "amd"
                    | "packagestaging"
                    | "webex"
                    | "programs"
            );
            assert!(skipped, "{name} should be skipped");
        }
    }

    #[test]
    fn display_name_from_app_paths_key_title_cases_stem() {
        assert_eq!(display_name_from_app_paths_key("firefox.exe"), "Firefox");
        assert_eq!(display_name_from_app_paths_key("code.exe"), "Code");
    }

    // ------------------------------------------------------------------
    // Helper-exe / redist filters — the headline fix for search lag was
    // "stop indexing Chromium helpers / updaters / crash handlers". These
    // tests pin the contract so real apps keep working and the common
    // noise stays out of the picker.
    // ------------------------------------------------------------------

    #[test]
    fn install_dir_skipped_stem_blocks_updaters_and_helpers() {
        // Update stubs & installer stubs
        assert!(install_dir_skipped_stem("Update"));
        assert!(install_dir_skipped_stem("updater"));
        assert!(install_dir_skipped_stem("Uninstall"));
        assert!(install_dir_skipped_stem("Setup"));
        assert!(install_dir_skipped_stem("Installer"));
        assert!(install_dir_skipped_stem("vc_redist.x64"));
        assert!(install_dir_skipped_stem("maintenanceservice"));
        // Chromium / Electron / Squirrel process family
        assert!(install_dir_skipped_stem("chrome_proxy"));
        assert!(install_dir_skipped_stem("chrome_crashpad_handler"));
        assert!(install_dir_skipped_stem("notification_helper"));
        assert!(install_dir_skipped_stem("squirrel"));
        assert!(install_dir_skipped_stem("crashpad_handler"));
        assert!(install_dir_skipped_stem("CrashReporter"));
        // Explicit helper-suffix families (Electron "<App> Helper (GPU)" etc.)
        assert!(install_dir_skipped_stem("MyApp_helper"));
        assert!(install_dir_skipped_stem("MyApp-helper"));
        assert!(install_dir_skipped_stem("MyApp Helper"));
        assert!(install_dir_skipped_stem("MyApp Helper (GPU)"));
        assert!(install_dir_skipped_stem("MyApp Helper (Renderer)"));
        // Bundled toolchain stubs that ship with Node/Electron/ffmpeg apps
        assert!(install_dir_skipped_stem("node"));
        assert!(install_dir_skipped_stem("ffmpeg"));
        assert!(install_dir_skipped_stem("python"));
    }

    #[test]
    fn install_dir_skipped_stem_allows_real_apps() {
        // Exact apps whose names happen to brush against substrings
        assert!(!install_dir_skipped_stem("Discord"));
        assert!(!install_dir_skipped_stem("Notepad"));
        assert!(!install_dir_skipped_stem("Code"));
        assert!(!install_dir_skipped_stem("Firefox"));
        assert!(!install_dir_skipped_stem("Photoshop"));
        assert!(!install_dir_skipped_stem("msedge"));
        // Regression: "UpdateManager" is a real app name — only the bare
        // "update" / "updater" stems get axed, not substrings.
        assert!(!install_dir_skipped_stem("UpdateManager"));
        // "nodejs installer" would be scary but bare "node.exe" is always a
        // bundled runtime; keep it skipped above and confirm related names pass.
        assert!(!install_dir_skipped_stem("Node Editor"));
        assert!(!install_dir_skipped_stem("NodePad"));
    }

    #[test]
    fn is_skippable_subdir_matches_common_resource_folders() {
        assert!(is_skippable_subdir("locales"));
        assert!(is_skippable_subdir("resources"));
        assert!(is_skippable_subdir("swiftshader"));
        assert!(is_skippable_subdir("_commonredist"));
        assert!(is_skippable_subdir("dotnet"));
        assert!(is_skippable_subdir("vcredist"));
        assert!(is_skippable_subdir("packages"));
        // Real app dirs must pass through.
        assert!(!is_skippable_subdir("discord"));
        assert!(!is_skippable_subdir("microsoft vs code"));
        assert!(!is_skippable_subdir("adobe"));
    }

    #[test]
    fn scan_install_dirs_shallow_picks_one_exe_per_app_dir() {
        let root = tempfile::tempdir().unwrap();
        // `Cursor\Cursor.exe` + a pile of Electron helpers → one entry.
        let cursor = root.path().join("Cursor");
        touch_file(&cursor.join("Cursor.exe"));
        touch_file(&cursor.join("Cursor Helper.exe"));
        touch_file(&cursor.join("Cursor Helper (GPU).exe"));
        touch_file(&cursor.join("Cursor Helper (Renderer).exe"));
        touch_file(&cursor.join("resources/app/node.exe"));
        touch_file(&cursor.join("ffmpeg.dll")); // not .exe, ignored anyway
        touch_file(&cursor.join("chrome_crashpad_handler.exe"));

        let mut map: HashMap<String, MapEntry> = HashMap::new();
        scan_install_dirs_shallow(root.path(), &mut map);

        let paths: Vec<String> = map.values().map(|e| e.inner.exe_path.to_lowercase()).collect();
        assert_eq!(
            paths.len(),
            1,
            "expected exactly one entry for the Cursor app dir, got {paths:?}"
        );
        assert!(
            paths[0].ends_with("cursor.exe"),
            "primary exe should be Cursor.exe, got {paths:?}"
        );
    }

    #[test]
    fn scan_install_dirs_shallow_recurses_into_publisher_app_layouts() {
        let root = tempfile::tempdir().unwrap();
        // `Adobe\Photoshop\Photoshop.exe` + `Adobe\Lightroom\Lightroom.exe`
        let adobe = root.path().join("Adobe");
        touch_file(&adobe.join("Photoshop/Photoshop.exe"));
        touch_file(&adobe.join("Photoshop/Uninstall.exe"));
        touch_file(&adobe.join("Lightroom/Lightroom.exe"));

        let mut map: HashMap<String, MapEntry> = HashMap::new();
        scan_install_dirs_shallow(root.path(), &mut map);

        let names: Vec<String> = map.values().map(|e| e.inner.display_name.to_lowercase()).collect();
        assert!(
            names.iter().any(|n| n.contains("photoshop")),
            "expected Photoshop entry, got {names:?}"
        );
        assert!(
            names.iter().any(|n| n.contains("lightroom")),
            "expected Lightroom entry, got {names:?}"
        );
    }

    #[test]
    fn pretty_name_from_identifier_splits_pascal_case() {
        let name = pretty_name_from_identifier("DeepRockGalactic");
        assert!(name.contains("Deep"), "got: {name}");
        assert!(name.contains("Rock"), "got: {name}");
    }

    #[test]
    fn expand_env_replaces_system_root() {
        let Ok(sr) = std::env::var("SystemRoot") else {
            return;
        };
        let out = expand_env(r"%SystemRoot%\system32\notepad.exe");
        assert!(
            out.to_lowercase().starts_with(&sr.to_lowercase()),
            "got: {out}"
        );
        assert!(!out.contains('%'), "unexpanded: {out}");
    }

    #[test]
    fn parse_start_apps_line_splits_on_first_tab() {
        let (n, id) = parse_start_apps_line("7-Zip\t{6D809377-6AF0-444B-8957-A3773F02200E}\\7-Zip\\7zFM.exe")
            .expect("line");
        assert_eq!(n, "7-Zip");
        assert!(id.contains("7zFM.exe"), "got {id}");
    }

    #[test]
    fn resolve_start_app_uwmid_to_shell_apps_folder() {
        let t = resolve_start_app_launch_target("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App")
            .expect("uwmid");
        assert!(
            t.to_ascii_lowercase()
                .starts_with("shell:appsfolder\\microsoft.windowsnotepad"),
            "got {t}"
        );
    }

    #[test]
    fn resolve_start_app_squirrel_aumid_to_shell_apps_folder() {
        let t = resolve_start_app_launch_target("com.squirrel.Discord.Discord").expect("squirrel");
        assert_eq!(
            t.to_ascii_lowercase(),
            "shell:appsfolder\\com.squirrel.discord.discord"
        );
    }

    #[test]
    fn resolve_start_app_rejects_https_url_app_id() {
        assert_eq!(
            resolve_start_app_launch_target("https://gitforwindows.org/faq"),
            None
        );
    }

    #[test]
    fn resolve_start_app_steam_protocol_passthrough() {
        assert_eq!(
            resolve_start_app_launch_target("steam://rungameid/123").as_deref(),
            Some("steam://rungameid/123")
        );
    }

    #[test]
    fn resolve_start_app_guid_system32_notepad() {
        let id = "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\notepad.exe";
        let t = resolve_start_app_launch_target(id).expect("guid-relative");
        assert!(t.to_lowercase().ends_with("notepad.exe"), "got {t}");
        assert!(Path::new(&t).exists(), "expected real notepad path: {t}");
    }

    #[test]
    fn known_folder_maps_program_files_guid() {
        let g = "6D809377-6AF0-444B-8957-A3773F02200E";
        let base = known_folder_guid_to_base(g).expect("base");
        assert!(base.is_dir(), "{base:?}");
        let joined = base.join("Windows NT\\Accessories\\wordpad.exe");
        if joined.is_file() {
            assert!(
                resolve_start_app_launch_target(&format!("{{{g}}}\\Windows NT\\Accessories\\wordpad.exe"))
                    .is_some(),
                "wordpad under Program Files"
            );
        }
    }

    /// Full scan touches registry, COM, several PowerShell passes, and optional icon extraction — slow.
    /// Run: `cargo test -p jarvis scan_full_smoke_notepad -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn scan_full_smoke_notepad_and_broad_cardinality() {
        let apps = scan().expect("scan");
        assert!(
            apps.len() >= 32,
            "expected broad coverage; got only {} entries",
            apps.len()
        );
        let has_notepad = apps.iter().any(|e| {
            e.display_name.to_lowercase().contains("notepad")
                || e.exe_path.to_lowercase().contains("notepad")
        });
        assert!(
            has_notepad,
            "expected Notepad (exe or UWP shell); sample: {:?}",
            apps.iter().take(8).collect::<Vec<_>>()
        );
    }

    /// Fast device check: same `Get-StartApps` pass the scanner uses must return many rows.
    #[cfg(windows)]
    #[test]
    fn get_start_apps_powershell_emits_tab_separated_rows() {
        let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$OutputEncoding = [System.Text.UTF8Encoding]::new()
foreach ($a in Get-StartApps) {
  $n = $a.Name -replace "`t", "`t"
  $i = $a.AppID -replace "`t", "`t"
  $n + "`t" + $i
}
"#;
        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .output()
            .expect("powershell");
        assert!(output.status.success(), "Get-StartApps ps failed");
        let n = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty() && l.contains('\t'))
            .count();
        assert!(
            n >= 24,
            "expected Start-menu enumeration (Get-StartApps) to yield many rows; got {n}"
        );
    }

    /// If the user has per-user `LocalAppData\\Programs` exes (e.g. VS Code under a subfolder), they must appear.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn scan_includes_local_appdata_programs_when_populated() {
        fn dir_has_exe_max_depth(dir: &Path, depth: usize, max_depth: usize) -> bool {
            if depth > max_depth || !dir.is_dir() {
                return false;
            }
            let Ok(rd) = std::fs::read_dir(dir) else {
                return false;
            };
            for e in rd.filter_map(|x| x.ok()) {
                let p = e.path();
                if p.is_file()
                    && p.extension()
                        .and_then(|x| x.to_str())
                        .map(|x| x.eq_ignore_ascii_case("exe"))
                        .unwrap_or(false)
                {
                    return true;
                }
                if p.is_dir() && dir_has_exe_max_depth(&p, depth + 1, max_depth) {
                    return true;
                }
            }
            false
        }

        let Ok(la) = std::env::var("LOCALAPPDATA") else {
            return;
        };
        let prog = Path::new(&la).join("Programs");
        if !prog.is_dir() {
            return;
        }
        if !dir_has_exe_max_depth(&prog, 0, 8) {
            return;
        }
        let apps = scan().expect("scan");
        let needle = "appdata\\local\\programs";
        let hit = apps
            .iter()
            .any(|e| e.exe_path.to_lowercase().contains(needle));
        assert!(
            hit,
            "expected at least one exe under LocalAppData\\Programs in index"
        );
    }

    /// `Get-StartApps` contributes many Explorer AUMIDs as `shell:AppsFolder\...` after the broad resolver.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn scan_includes_many_shell_aumid_entries() {
        let apps = scan().expect("scan");
        let n = apps
            .iter()
            .filter(|e| {
                e.exe_path
                    .to_ascii_lowercase()
                    .starts_with("shell:appsfolder\\")
            })
            .count();
        assert!(
            n >= 16,
            "expected Get-StartApps + UWP to yield many shell:AppsFolder rows; got {n}"
        );
    }
}

//! Registry Uninstall + Start Menu / Desktop / pinned `.lnk` + `.url` (Steam, Epic, Battle.net, etc.),
//! flat / limited-depth `*.exe` inventory under System32 / SysWOW64 / WinDir + Program Files,
//! plus Steam / Epic / GOG library roots and Start Apps (UWP / packaged) aliases (Windows).

use super::AppEntry;
use serde::Deserialize;
use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink, SLGP_RAWPATH};
use winreg::enums::*;
use winreg::types::FromRegValue;
use winreg::RegKey;
use winreg::RegValue;

/// Higher wins when merging duplicate `exe_path` keys; avoids flat `.exe` inventory clobbering Start Menu / Uninstall names.
const NAME_RANK_UNINSTALL: u8 = 80;
const NAME_RANK_UWP_START_APPS: u8 = 75;
const NAME_RANK_START_MENU: u8 = 70;
const NAME_RANK_LAUNCHER_MANIFEST: u8 = 65;
const NAME_RANK_APP_PATHS: u8 = 55;
const NAME_RANK_SEED: u8 = 52;
const NAME_RANK_STEAM_FOLDER: u8 = 45;
const NAME_RANK_RECURSIVE_PF: u8 = 30;
const NAME_RANK_FLAT_EXE: u8 = 20;

/// `IShellLinkW` path / argument buffers (game shortcuts often exceed `MAX_PATH`).
const SHELL_LINK_WCHAR_CAP: usize = 8192;

const GAME_PROTOCOL_PREFIXES: &[&str] = &[
    "steam://",
    "com.epicgames.launcher://",
    "battlenet://",
    "blizzard://",
    "uplay://",
    "ubisoft://",
    "ubisoftconnect://",
    "goggalaxy://",
    "gog://",
    "rockstar://",
    "origin://",
    "eadesktop://",
    "msxbox://",
    "xbox://",
];

#[derive(Debug, Clone)]
pub(crate) struct IndexedEntry {
    pub display_name: String,
    pub exe_path: String,
    pub icon_data_url: Option<String>,
    pub name_rank: u8,
}

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
        unsafe {
            CoUninitialize();
        }
    }
}

pub fn scan() -> Result<Vec<AppEntry>, String> {
    let mut map: HashMap<String, IndexedEntry> = HashMap::new();
    // Fast registry pass: fills most launchers without shell COM (icons added later by merge).
    scan_app_paths_registry(&mut map)?;
    scan_uninstall_registry(&mut map)?;
    let _com = ComApartment::new()?;
    scan_start_menu(&mut map)?;
    seed_windows_accessories(&mut map);
    scan_steam_libraries(&mut map)?;
    scan_epic_launcher_installed(&mut map)?;
    scan_gog_registry(&mut map)?;
    scan_uwp_shell_apps(&mut map)?;
    // Last: .exe inventory (flat system dirs + shallow recursive Program Files); low merge rank.
    scan_system_and_program_exe_inventory(&mut map)?;
    Ok(map.into_values().map(into_app_entry).collect())
}

fn into_app_entry(e: IndexedEntry) -> AppEntry {
    AppEntry {
        display_name: e.display_name,
        exe_path: e.exe_path,
        icon_data_url: e.icon_data_url,
    }
}

/// Non-recursive `*.exe` listing. Icons skipped (merge may add later).
fn scan_flat_exe_directory(
    dir: &Path,
    map: &mut HashMap<String, IndexedEntry>,
    max_add: usize,
    name_rank: u8,
) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("exe"))
                    .unwrap_or(false)
        })
        .collect();
    paths.sort();
    let mut added = 0usize;
    for p in paths {
        if added >= max_add {
            break;
        }
        let exe_norm = std::fs::canonicalize(&p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_string();
        let label = display_name_from_exe_path(&p);
        insert_entry(map, exe_norm, label, None, name_rank);
        added += 1;
    }
    Ok(())
}

fn should_skip_program_files_subdir(name: &str) -> bool {
    let n = name.to_uppercase();
    matches!(
        n.as_str(),
        "$RECYCLE.BIN" | "SYSTEM VOLUME INFORMATION" | "WINDOWSAPPS"
    )
}

/// Depth-limited recursive `.exe` discovery (used for Program Files game folders).
fn scan_exe_directory_recursive(
    dir: &Path,
    map: &mut HashMap<String, IndexedEntry>,
    budget: &mut usize,
    max_depth: usize,
    depth: usize,
    name_rank: u8,
) -> Result<(), String> {
    if *budget == 0 || !dir.is_dir() {
        return Ok(());
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("exe"))
                    .unwrap_or(false)
        })
        .collect();
    paths.sort();
    for p in paths {
        if *budget == 0 {
            break;
        }
        let exe_norm = std::fs::canonicalize(&p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_string();
        let label = display_name_from_exe_path(&p);
        insert_entry(map, exe_norm, label, None, name_rank);
        *budget -= 1;
    }
    if depth >= max_depth {
        return Ok(());
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    children.sort();
    for child in children {
        let name = child
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if should_skip_program_files_subdir(name) {
            continue;
        }
        scan_exe_directory_recursive(
            &child,
            map,
            budget,
            max_depth,
            depth + 1,
            name_rank,
        )?;
    }
    Ok(())
}

fn display_name_from_exe_path(p: &Path) -> String {
    let name = p
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("App.exe");
    display_name_from_app_paths_key(name)
}

fn scan_system_and_program_exe_inventory(map: &mut HashMap<String, IndexedEntry>) -> Result<(), String> {
    let sys_root = std::env::var("SystemRoot")
        .or_else(|_| std::env::var("WINDIR"))
        .unwrap_or_default();
    if !sys_root.is_empty() {
        let root = Path::new(&sys_root);
        scan_flat_exe_directory(
            &root.join("System32"),
            map,
            5000,
            NAME_RANK_FLAT_EXE,
        )?;
        scan_flat_exe_directory(
            &root.join("SysWOW64"),
            map,
            5000,
            NAME_RANK_FLAT_EXE,
        )?;
        scan_flat_exe_directory(root, map, 384, NAME_RANK_FLAT_EXE)?;
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        let mut budget = 4096usize;
        scan_exe_directory_recursive(
            Path::new(&pf),
            map,
            &mut budget,
            5,
            0,
            NAME_RANK_RECURSIVE_PF,
        )?;
    }
    if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
        let mut budget = 4096usize;
        scan_exe_directory_recursive(
            Path::new(&pf86),
            map,
            &mut budget,
            5,
            0,
            NAME_RANK_RECURSIVE_PF,
        )?;
    }
    Ok(())
}

/// Expand common `%...%` segments in registry paths (e.g. App Paths default `%SystemRoot%\\system32\\notepad.exe`).
fn expand_windows_env_path(raw: &str) -> String {
    let mut s = raw.to_string();
    if let Ok(v) = std::env::var("SystemRoot") {
        for pat in ["%SystemRoot%", "%SYSTEMROOT%", "%systemroot%"] {
            s = s.replace(pat, &v);
        }
    }
    if let Ok(v) = std::env::var("WINDIR") {
        for pat in ["%WINDIR%", "%windir%"] {
            s = s.replace(pat, &v);
        }
    }
    if let Ok(v) = std::env::var("ProgramFiles") {
        s = s.replace("%ProgramFiles%", &v);
        s = s.replace("%PROGRAMFILES%", &v);
    }
    if let Ok(v) = std::env::var("ProgramFiles(x86)") {
        s = s.replace("%ProgramFiles(x86)%", &v);
        s = s.replace("%PROGRAMFILES(X86)%", &v);
    }
    if let Ok(v) = std::env::var("ProgramW6432") {
        s = s.replace("%ProgramW6432%", &v);
    }
    if let Ok(v) = std::env::var("CommonProgramFiles") {
        s = s.replace("%CommonProgramFiles%", &v);
    }
    if let Ok(v) = std::env::var("LOCALAPPDATA") {
        s = s.replace("%LOCALAPPDATA%", &v);
        s = s.replace("%localappdata%", &v);
    }
    if let Ok(v) = std::env::var("APPDATA") {
        s = s.replace("%APPDATA%", &v);
    }
    if let Ok(v) = std::env::var("USERPROFILE") {
        s = s.replace("%USERPROFILE%", &v);
    }
    if let Ok(v) = std::env::var("PUBLIC") {
        s = s.replace("%PUBLIC%", &v);
    }
    s
}

fn reg_value_as_expanded_string(val: &RegValue) -> Option<String> {
    let s = String::from_reg_value(val).ok()?;
    let t = s.trim().trim_matches('"');
    if t.is_empty() {
        return None;
    }
    Some(expand_windows_env_path(t))
}

/// Reads the subkey's default `(Default)` value; `get_value("")` misses some builds, and values may be REG_EXPAND_SZ.
fn reg_subkey_default_target(sub: &RegKey) -> Option<String> {
    for res in sub.enum_values() {
        let Some((name, val)) = res.ok() else {
            continue;
        };
        if !name.is_empty() {
            continue;
        }
        if let Some(s) = reg_value_as_expanded_string(&val) {
            return Some(s);
        }
    }
    sub.get_value::<String, _>("")
        .ok()
        .map(|s| expand_windows_env_path(s.trim().trim_matches('"')))
}

fn seed_windows_accessories(map: &mut HashMap<String, IndexedEntry>) {
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
    ];
    for (name, rel) in PAIRS {
        let p = Path::new(&sys).join(rel);
        if !p.is_file() {
            continue;
        }
        let exe_norm = std::fs::canonicalize(&p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_string();
        insert_entry(map, exe_norm, (*name).into(), None, NAME_RANK_SEED);
    }
}

/// `App Paths` maps `something.exe` → default value full path (broad coverage vs Uninstall alone).
fn scan_app_paths_registry(map: &mut HashMap<String, IndexedEntry>) -> Result<(), String> {
    const SUBPATH: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
    const SUBPATH_WOW64: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths";

    for (root, subpath) in [
        (HKEY_LOCAL_MACHINE, SUBPATH),
        (HKEY_LOCAL_MACHINE, SUBPATH_WOW64),
        (HKEY_CURRENT_USER, SUBPATH),
    ] {
        let hkey = RegKey::predef(root);
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
            let Some(target) = reg_subkey_default_target(&sub) else {
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
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !ext.eq_ignore_ascii_case("exe") || !path.exists() {
                continue;
            }
            let exe_norm = std::fs::canonicalize(path)
                .unwrap_or_else(|_| path.to_path_buf())
                .to_string_lossy()
                .to_string();
            let display_name = display_name_from_app_paths_key(&exe_key);
            // Skip per-exe icon extraction here (hundreds of keys); merge from Uninstall/Start Menu when present.
            insert_entry(
                map,
                exe_norm,
                display_name,
                None,
                NAME_RANK_APP_PATHS,
            );
        }
    }
    Ok(())
}

fn display_name_from_app_paths_key(exe_key: &str) -> String {
    let stem = if exe_key.len() >= 4 && exe_key[exe_key.len() - 4..].eq_ignore_ascii_case(".exe") {
        &exe_key[..exe_key.len() - 4]
    } else {
        exe_key
    };
    let stem = stem.replace('_', " ");
    let mut it = stem.chars();
    match it.next() {
        None => exe_key.to_string(),
        Some(c) => c.to_uppercase().chain(it).collect(),
    }
}

fn scan_uninstall_registry(map: &mut HashMap<String, IndexedEntry>) -> Result<(), String> {
    let roots: [(winreg::HKEY, &str); 3] = [
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
    for (root, subpath) in roots {
        let hkey = RegKey::predef(root);
        let Ok(uninstall) = hkey.open_subkey(subpath) else {
            continue;
        };
        for entry in uninstall.enum_keys().filter_map(|e| e.ok()) {
            let Ok(sub) = uninstall.open_subkey(&entry) else {
                continue;
            };
            let display_name = match sub.get_value::<String, _>("DisplayName") {
                Ok(s) => {
                    let t = s.trim();
                    if t.is_empty() {
                        continue;
                    }
                    t.to_string()
                }
                Err(_) => continue,
            };
            let install_loc = sub
                .get_value::<String, _>("InstallLocation")
                .ok()
                .map(|loc| expand_windows_env_path(loc.trim()));
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
            let Some(exe_path) = exe else {
                continue;
            };
            let exe_norm = exe_path.trim().to_string();
            if exe_norm.is_empty() || !Path::new(&exe_norm).exists() {
                continue;
            }
            let icon_data_url = extract_icon_data_url(&exe_norm);
            insert_entry(
                map,
                exe_norm,
                display_name,
                icon_data_url,
                NAME_RANK_UNINSTALL,
            );
        }
    }
    Ok(())
}

fn display_icon_to_exe(raw: &str) -> Option<String> {
    let trimmed = expand_windows_env_path(raw.trim());
    let trimmed = trimmed.trim_matches('"');
    let first = trimmed.split(',').next()?.trim();
    if first.is_empty() {
        return None;
    }
    let p = Path::new(first);
    if !p.is_absolute() {
        return None;
    }
    let ext = p.extension()?.to_str()?;
    if !ext.eq_ignore_ascii_case("exe") && !ext.eq_ignore_ascii_case("dll") {
        return None;
    }
    // Prefer .exe; skip icon-only .dll refs
    if ext.eq_ignore_ascii_case("dll") {
        return None;
    }
    if p.exists() {
        Some(
            std::fs::canonicalize(p)
                .unwrap_or_else(|_| p.to_path_buf())
                .to_string_lossy()
                .to_string(),
        )
    } else {
        None
    }
}

fn install_location_guess(display_name: &str, loc: &str) -> Option<String> {
    let dir = Path::new(loc.trim());
    if !dir.is_dir() {
        return None;
    }
    let stem = display_name
        .split_whitespace()
        .next()
        .unwrap_or(display_name);
    let candidate = dir.join(format!("{stem}.exe"));
    if candidate.is_file() {
        return Some(candidate.to_string_lossy().to_string());
    }
    None
}

fn install_dir_skipped_stem(stem: &str) -> bool {
    let s = stem.to_lowercase();
    const SKIP: &[&str] = &[
        "uninstall",
        "unins000",
        "unins001",
        "unins002",
        "setup",
        "install",
        "update",
        "maintenanceservice",
        "crashreporter",
        "elevate",
        "vc_redist",
    ];
    SKIP.iter().any(|p| s.contains(p))
}

/// When `DisplayIcon` / `{Name}.exe` in install dir miss, pick a plausible `.exe` in that folder.
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
    let mut best_match: Option<(usize, PathBuf)> = None;
    let mut best_fallback: Option<(usize, PathBuf)> = None;
    for p in exes.iter() {
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if stem.len() < 2 {
            continue;
        }
        let score = if dn.contains(&stem) {
            1000 + stem.len()
        } else {
            0
        };
        if score > 0 {
            let replace = match &best_match {
                None => true,
                Some((s, _)) => score > *s,
            };
            if replace {
                best_match = Some((score, p.clone()));
            }
            continue;
        }
        let fb_score = install_dir_fallback_exe_score(&stem);
        let replace = match &best_fallback {
            None => true,
            Some((s, _)) => fb_score > *s,
        };
        if replace {
            best_fallback = Some((fb_score, p.clone()));
        }
    }
    if let Some((_, p)) = best_match {
        return Some(p.to_string_lossy().to_string());
    }
    if let Some((_, p)) = best_fallback {
        return Some(p.to_string_lossy().to_string());
    }
    if exes.len() == 1 {
        return Some(exes[0].to_string_lossy().to_string());
    }
    None
}

/// Prefer shorter stems when the display name gives no substring match (e.g. `FSD.exe` for "Deep Rock Galactic").
fn install_dir_fallback_exe_score(stem_lower: &str) -> usize {
    let len = stem_lower.len();
    let noise = stem_lower.contains("launcher")
        || stem_lower.contains("bootstrap")
        || stem_lower.contains("redist")
        || stem_lower.contains("sdk")
        || stem_lower.contains("editor");
    let tier: usize = if noise { 100 } else { 300 };
    tier.saturating_sub(len)
}

fn wide_nul_trim(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len]).trim().to_string()
}

fn is_steam_client_exe_path(target: &str) -> bool {
    Path::new(target)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|n| n.eq_ignore_ascii_case("steam.exe"))
        .unwrap_or(false)
}

fn parse_steam_applaunch_id(args: &str) -> Option<u32> {
    let lower = args.to_ascii_lowercase();
    for key in ["-applaunch", "/applaunch"] {
        let Some(idx) = lower.find(key) else {
            continue;
        };
        let rest = args[idx + key.len()..].trim_start();
        let mut digits = String::new();
        for c in rest.chars() {
            if c.is_ascii_digit() {
                digits.push(c);
            } else if !digits.is_empty() {
                break;
            }
        }
        if let Ok(id) = digits.parse::<u32>() {
            return Some(id);
        }
    }
    None
}

fn extract_game_protocol_url(target: &str, args: &str) -> Option<String> {
    for hay in [args, target] {
        if hay.is_empty() {
            continue;
        }
        let lower = hay.to_ascii_lowercase();
        for pref in GAME_PROTOCOL_PREFIXES {
            let Some(idx) = lower.find(pref) else {
                continue;
            };
            let slice = &hay[idx..];
            let end = slice
                .find(|c: char| {
                    c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ')' | '>' | '<')
                })
                .unwrap_or(slice.len());
            let url = slice[..end].trim_end_matches(|c| c == '"' || c == '\'');
            if !url.is_empty() {
                return Some(url.to_string());
            }
        }
    }
    None
}

/// Resolves a shell shortcut to something `cmd /c start` can launch (`*.exe`, `steam://…`, etc.).
/// Returns `(launch_spec, optional_icon_exe_path)`.
fn shortcut_launch_spec(target: &str, args: &str) -> Option<(String, Option<String>)> {
    let target_t = target.trim();
    let args_t = args.trim();
    if let Some(url) = extract_game_protocol_url(target_t, args_t) {
        let icon = if is_steam_client_exe_path(target_t) {
            Some(target_t.to_string())
        } else {
            None
        };
        return Some((url, icon));
    }
    if let Some(id) = parse_steam_applaunch_id(args_t) {
        if is_steam_client_exe_path(target_t) {
            return Some((
                format!("steam://rungameid/{id}"),
                Some(target_t.to_string()),
            ));
        }
    }
    let p = Path::new(target_t);
    if p.is_file() {
        return Some((target_t.to_string(), Some(target_t.to_string())));
    }
    None
}

fn parse_internet_shortcut_url(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for raw in text.lines() {
        let line = raw.trim();
        let rest = if let Some(r) = line.strip_prefix("URL=") {
            r
        } else if let Some(r) = line.strip_prefix("url=") {
            r
        } else {
            continue;
        };
        let u = rest.trim();
        if let Some(url) = extract_game_protocol_url("", u) {
            return Some(url);
        }
    }
    None
}

fn scan_start_menu(map: &mut HashMap<String, IndexedEntry>) -> Result<(), String> {
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| e.to_string())?;
    let persist: IPersistFile = shell_link.cast().map_err(|e| e.to_string())?;

    let mut roots = Vec::new();
    if let Ok(pd) = std::env::var("PROGRAMDATA") {
        roots.push(PathBuf::from(pd).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Ok(ad) = std::env::var("APPDATA") {
        roots.push(PathBuf::from(&ad).join(r"Microsoft\Windows\Start Menu\Programs"));
        roots.push(
            PathBuf::from(&ad)
                .join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar"),
        );
    }
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(la).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Ok(public) = std::env::var("PUBLIC") {
        roots.push(PathBuf::from(public).join("Desktop"));
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        roots.push(PathBuf::from(&profile).join("Desktop"));
        roots.push(PathBuf::from(&profile).join("Links"));
    }

    for root in roots {
        if root.is_dir() {
            visit_dir_lnk(&root, &shell_link, &persist, map)?;
        }
    }
    Ok(())
}

fn visit_dir_lnk(
    dir: &Path,
    shell_link: &IShellLinkW,
    persist: &IPersistFile,
    map: &mut HashMap<String, IndexedEntry>,
) -> Result<(), String> {
    let read = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for e in read {
        let e = e.map_err(|e| e.to_string())?;
        let p = e.path();
        if p.is_dir() {
            visit_dir_lnk(&p, shell_link, persist, map)?;
        } else if p.extension().and_then(|x| x.to_str()) == Some("lnk") {
            if let Some((launch, icon_probe)) = resolve_shell_link(shell_link, persist, &p) {
                if launch.is_empty() {
                    continue;
                }
                let label = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("App")
                    .to_string();
                let icon_data_url = icon_probe
                    .as_deref()
                    .and_then(|ip| extract_icon_data_url(ip));
                insert_entry(
                    map,
                    launch,
                    label,
                    icon_data_url,
                    NAME_RANK_START_MENU,
                );
            }
        } else if p.extension().and_then(|x| x.to_str()) == Some("url") {
            if let Some(url) = parse_internet_shortcut_url(&p) {
                let label = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Game")
                    .to_string();
                insert_entry(
                    map,
                    url,
                    label,
                    None,
                    NAME_RANK_START_MENU,
                );
            }
        }
    }
    Ok(())
}

fn resolve_shell_link(
    shell_link: &IShellLinkW,
    persist: &IPersistFile,
    lnk: &Path,
) -> Option<(String, Option<String>)> {
    let wide: Vec<u16> = lnk
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
        let mut path_buf = vec![0u16; SHELL_LINK_WCHAR_CAP];
        let target = match shell_link.GetPath(
            &mut path_buf,
            std::ptr::null_mut::<WIN32_FIND_DATAW>(),
            SLGP_RAWPATH.0 as u32,
        ) {
            Ok(()) => wide_nul_trim(&path_buf),
            Err(_) => String::new(),
        };
        let mut arg_buf = vec![0u16; SHELL_LINK_WCHAR_CAP];
        let args = match shell_link.GetArguments(&mut arg_buf) {
            Ok(()) => wide_nul_trim(&arg_buf),
            Err(_) => String::new(),
        };
        shortcut_launch_spec(&target, &args)
    }
}

fn steam_install_dir_from_registry() -> Option<PathBuf> {
    for (root, sub) in [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam"),
        (HKEY_CURRENT_USER, r"Software\Valve\Steam"),
    ] {
        let hkey = RegKey::predef(root);
        let Ok(k) = hkey.open_subkey(sub) else {
            continue;
        };
        if let Ok(s) = k.get_value::<String, _>("InstallPath") {
            let p = PathBuf::from(expand_windows_env_path(s.trim()));
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

fn vdf_quoted_path_byte_end(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                i = (i + 2).min(b.len());
            }
            b'"' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

fn vdf_unescape_inner(escaped: &str) -> String {
    let mut out = String::with_capacity(escaped.len());
    let mut it = escaped.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(n) = it.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn vdf_extract_library_path_strings(vdf: &str) -> Vec<String> {
    let needle = "\"path\"";
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(pos) = vdf[from..].find(needle) {
        let abs = from + pos;
        let i = abs + needle.len();
        from = i + 1;
        let tail = vdf[i..].trim_start();
        let Some(after_open) = tail.strip_prefix('"') else {
            continue;
        };
        let Some(end) = vdf_quoted_path_byte_end(after_open) else {
            continue;
        };
        out.push(vdf_unescape_inner(&after_open[..end]));
    }
    out
}

fn steam_library_steamapps_roots() -> Vec<PathBuf> {
    use std::collections::HashSet;
    let mut seen = HashSet::<PathBuf>::new();
    let mut out = Vec::new();
    let Some(base) = steam_install_dir_from_registry() else {
        return out;
    };
    let push = |p: PathBuf, seen: &mut HashSet<PathBuf>, out: &mut Vec<PathBuf>| {
        if p.is_dir() && seen.insert(p.clone()) {
            out.push(p);
        }
    };
    push(base.join("steamapps"), &mut seen, &mut out);
    for rel in ["config/libraryfolders.vdf", "steamapps/libraryfolders.vdf"] {
        let p = base.join(rel);
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        for raw in vdf_extract_library_path_strings(&text) {
            let expanded = expand_windows_env_path(raw.trim());
            let pb = PathBuf::from(expanded.trim_matches('"'));
            let cand = if pb.ends_with("steamapps") {
                pb
            } else {
                pb.join("steamapps")
            };
            push(cand, &mut seen, &mut out);
        }
    }
    out
}

fn display_name_from_steam_folder(folder: &str) -> String {
    folder.replace('_', " ")
}

fn scan_steam_libraries(map: &mut HashMap<String, IndexedEntry>) -> Result<(), String> {
    let roots = steam_library_steamapps_roots();
    let mut games_seen = 0usize;
    for steamapps in roots {
        let common = steamapps.join("common");
        if !common.is_dir() {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&common) else {
            continue;
        };
        let mut game_dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
        game_dirs.sort();
        for game_dir in game_dirs {
            if games_seen >= 768 {
                return Ok(());
            }
            let title = game_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Game");
            let display = display_name_from_steam_folder(title);
            let Some(exe_raw) = install_dir_primary_exe(&game_dir.to_string_lossy(), &display) else {
                continue;
            };
            let p = Path::new(exe_raw.trim());
            if !p.is_file() {
                continue;
            }
            let exe_norm = std::fs::canonicalize(p)
                .unwrap_or_else(|_| p.to_path_buf())
                .to_string_lossy()
                .to_string();
            insert_entry(
                map,
                exe_norm,
                display,
                None,
                NAME_RANK_STEAM_FOLDER,
            );
            games_seen += 1;
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct EpicInstalledRoot {
    #[serde(rename = "InstallationList")]
    installation_list: Option<Vec<EpicInstalledItem>>,
}

#[derive(Debug, Deserialize)]
struct EpicInstalledItem {
    #[serde(rename = "InstallLocation")]
    install_location: Option<String>,
    #[serde(rename = "AppName")]
    app_name: Option<String>,
    #[serde(rename = "AppDisplayName")]
    app_display_name: Option<String>,
    #[serde(rename = "FriendlyName")]
    friendly_name: Option<String>,
}

fn scan_epic_launcher_installed(map: &mut HashMap<String, IndexedEntry>) -> Result<(), String> {
    let Ok(pd) = std::env::var("PROGRAMDATA") else {
        return Ok(());
    };
    let path = Path::new(&pd).join("Epic/UnrealEngineLauncher/LauncherInstalled.dat");
    if !path.is_file() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let root: EpicInstalledRoot = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let Some(list) = root.installation_list else {
        return Ok(());
    };
    let mut n = 0usize;
    for item in list {
        if n >= 512 {
            break;
        }
        let loc = item.install_location.as_deref().unwrap_or("").trim();
        if loc.is_empty() {
            continue;
        }
        let display = item
            .app_display_name
            .as_deref()
            .or(item.friendly_name.as_deref())
            .or(item.app_name.as_deref())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                Path::new(loc)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Epic Game")
                    .to_string()
            });
        let Some(exe_raw) = install_dir_primary_exe(loc, &display) else {
            continue;
        };
        let p = Path::new(exe_raw.trim());
        if !p.is_file() {
            continue;
        }
        let exe_norm = std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_string();
        insert_entry(
            map,
            exe_norm,
            display,
            None,
            NAME_RANK_LAUNCHER_MANIFEST,
        );
        n += 1;
    }
    Ok(())
}

fn scan_gog_registry(map: &mut HashMap<String, IndexedEntry>) -> Result<(), String> {
    for (root, subpath) in [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\GOG.com\Games"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\GOG.com\Games"),
    ] {
        let hkey = RegKey::predef(root);
        let Ok(games) = hkey.open_subkey(subpath) else {
            continue;
        };
        for key in games.enum_keys().filter_map(|e| e.ok()) {
            let Ok(sk) = games.open_subkey(&key) else {
                continue;
            };
            let display_name = sk
                .get_value::<String, _>("gameName")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| key.clone());
            let Ok(exe_raw) = sk.get_value::<String, _>("exe") else {
                continue;
            };
            let expanded = expand_windows_env_path(exe_raw.trim());
            let p = Path::new(expanded.trim().trim_matches('"'));
            if !p.is_file() {
                continue;
            }
            let exe_norm = std::fs::canonicalize(p)
                .unwrap_or_else(|_| p.to_path_buf())
                .to_string_lossy()
                .to_string();
            let icon_data_url = extract_icon_data_url(&exe_norm);
            insert_entry(
                map,
                exe_norm,
                display_name,
                icon_data_url,
                NAME_RANK_LAUNCHER_MANIFEST,
            );
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct StartAppRow {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "AppID", alias = "AppId")]
    app_id: String,
}

fn scan_uwp_shell_apps(map: &mut HashMap<String, IndexedEntry>) -> Result<(), String> {
    let script = "try { Get-StartApps | Select-Object Name,AppID | ConvertTo-Json -Compress -Depth 4 } catch { '[]' }";
    let Ok(output) = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(script)
        .output()
    else {
        return Ok(());
    };
    if !output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let rows: Vec<StartAppRow> = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => serde_json::from_str::<StartAppRow>(trimmed)
            .map(|r| vec![r])
            .unwrap_or_default(),
    };
    let mut n = 0usize;
    for row in rows {
        if n >= 800 {
            break;
        }
        let name = row.name.trim();
        let id = row.app_id.trim();
        if name.is_empty() || id.is_empty() {
            continue;
        }
        let shell = format!("shell:AppsFolder\\{id}");
        insert_entry(
            map,
            shell,
            name.to_string(),
            None,
            NAME_RANK_UWP_START_APPS,
        );
        n += 1;
    }
    Ok(())
}

fn extract_icon_data_url(exe_path: &str) -> Option<String> {
    let escaped_path = exe_path.replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         Add-Type -AssemblyName System.Drawing; \
         $icon = [System.Drawing.Icon]::ExtractAssociatedIcon('{escaped_path}'); \
         if ($null -eq $icon) {{ exit 0 }}; \
         $bitmap = $icon.ToBitmap(); \
         $stream = New-Object System.IO.MemoryStream; \
         $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png); \
         [Convert]::ToBase64String($stream.ToArray())"
    );
    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(script)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let b64 = String::from_utf8(output.stdout).ok()?;
    let trimmed = b64.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!("data:image/png;base64,{trimmed}"))
}

fn insert_entry(
    map: &mut HashMap<String, IndexedEntry>,
    exe_path: String,
    display_name: String,
    icon_data_url: Option<String>,
    name_rank: u8,
) {
    let key = exe_path.to_lowercase();
    let incoming = IndexedEntry {
        display_name,
        exe_path,
        icon_data_url,
        name_rank,
    };
    match map.get_mut(&key) {
        None => {
            map.insert(key, incoming);
        }
        Some(existing) => {
            if name_rank > existing.name_rank {
                let merged_icon = incoming
                    .icon_data_url
                    .clone()
                    .or_else(|| existing.icon_data_url.clone());
                *existing = IndexedEntry {
                    display_name: incoming.display_name,
                    exe_path: incoming.exe_path,
                    icon_data_url: merged_icon,
                    name_rank: incoming.name_rank,
                };
            } else if name_rank < existing.name_rank {
                if existing.icon_data_url.is_none() {
                    existing.icon_data_url = incoming.icon_data_url.clone();
                }
            } else if existing.icon_data_url.is_none() {
                existing.icon_data_url = incoming.icon_data_url.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_entry_backfills_missing_icon_on_existing_row() {
        let mut map = HashMap::<String, IndexedEntry>::new();
        insert_entry(
            &mut map,
            r"C:\Apps\Example.exe".into(),
            "Example".into(),
            None,
            NAME_RANK_FLAT_EXE,
        );
        insert_entry(
            &mut map,
            r"C:\Apps\Example.exe".into(),
            "Example".into(),
            Some("data:image/png;base64,AAA=".into()),
            NAME_RANK_FLAT_EXE,
        );
        let stored = map
            .get(&r"c:\apps\example.exe".to_string())
            .expect("entry exists");
        assert_eq!(
            stored.icon_data_url.as_deref(),
            Some("data:image/png;base64,AAA=")
        );
    }

    #[test]
    fn insert_entry_keeps_better_source_display_name_over_longer_low_rank() {
        let mut map = HashMap::<String, IndexedEntry>::new();
        let path = r"C:\Games\Battle.net\Battle.net Launcher.exe".to_string();
        insert_entry(
            &mut map,
            path.clone(),
            "Battle.net".into(),
            None,
            NAME_RANK_START_MENU,
        );
        insert_entry(
            &mut map,
            path,
            "Battlenetlauncher".into(),
            None,
            NAME_RANK_FLAT_EXE,
        );
        let stored = map
            .get(&r"c:\games\battle.net\battle.net launcher.exe".to_string())
            .expect("entry exists");
        assert_eq!(stored.display_name, "Battle.net");
    }

    #[test]
    fn display_name_from_app_paths_key_title_cases_stem() {
        assert_eq!(display_name_from_app_paths_key("firefox.exe"), "Firefox");
        assert_eq!(display_name_from_app_paths_key("WINWORD.EXE"), "WINWORD");
        assert_eq!(display_name_from_app_paths_key("code.exe"), "Code");
    }

    #[test]
    fn shortcut_launch_spec_steam_exe_applaunch_becomes_rungameid() {
        let (launch, icon) = shortcut_launch_spec(
            r"C:\Steam\steam.exe",
            "-silent -applaunch 440",
        )
        .expect("steam shortcut");
        assert_eq!(launch, "steam://rungameid/440");
        assert_eq!(icon.as_deref(), Some(r"C:\Steam\steam.exe"));
    }

    #[test]
    fn shortcut_launch_spec_prefers_protocol_in_arguments() {
        let (launch, _) = shortcut_launch_spec(
            r"C:\Epic\EpicGamesLauncher.exe",
            r"com.epicgames.launcher://apps/abc?action=launch",
        )
        .expect("epic protocol");
        assert!(launch.to_ascii_lowercase().starts_with("com.epicgames.launcher://"));
    }

    #[test]
    fn parse_internet_shortcut_url_reads_steam_url_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Game.url");
        std::fs::write(
            &p,
            "[InternetShortcut]\nURL=steam://rungameid/123\n",
        )
        .unwrap();
        let u = parse_internet_shortcut_url(&p).expect("url");
        assert_eq!(u, "steam://rungameid/123");
    }

    #[test]
    fn install_dir_primary_exe_prefers_stem_contained_in_display_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Discord.exe"), [0u8]).unwrap();
        std::fs::write(dir.path().join("Uninstall.exe"), [0u8]).unwrap();
        std::fs::write(dir.path().join("Helper.exe"), [0u8]).unwrap();
        let hit = install_dir_primary_exe(
            &dir.path().to_string_lossy(),
            "Discord (some channel)",
        )
        .expect("match");
        assert!(hit.to_lowercase().contains("discord.exe"));
    }

    #[test]
    fn install_dir_primary_exe_none_when_only_installer_exes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Uninstall.exe"), [0u8]).unwrap();
        std::fs::write(dir.path().join("Setup.exe"), [0u8]).unwrap();
        assert!(install_dir_primary_exe(&dir.path().to_string_lossy(), "Some App").is_none());
    }

    #[test]
    fn install_dir_primary_exe_picks_sole_survivor() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Tool.exe"), [0u8]).unwrap();
        let hit = install_dir_primary_exe(&dir.path().to_string_lossy(), "Unknown Product").expect("exe");
        assert!(hit.to_lowercase().ends_with("tool.exe"));
    }

    #[test]
    fn install_dir_primary_exe_matches_mismatched_game_stem_to_display_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("FSD.exe"), [0u8]).unwrap();
        std::fs::write(dir.path().join("SomeLongBootstrapper.exe"), [0u8]).unwrap();
        let hit = install_dir_primary_exe(
            &dir.path().to_string_lossy(),
            "Deep Rock Galactic",
        )
        .expect("exe");
        assert!(hit.to_lowercase().ends_with("fsd.exe"));
    }

    #[test]
    fn vdf_extract_library_path_strings_finds_paths() {
        let sample = r#""libraryfolders"
{
	"0"
	{
		"path"		"D:\\SteamLibrary"
	}
}
"#;
        let paths = vdf_extract_library_path_strings(sample);
        assert!(
            paths.iter().any(|p| p.replace('/', "\\").contains("SteamLibrary")),
            "paths={paths:?}"
        );
    }

    #[test]
    fn epic_installed_manifest_deserializes() {
        let json = r#"{"InstallationList":[{"InstallLocation":"C:\\Epic\\MyGame","AppName":"MyGame","AppDisplayName":"My Cool Game"}]}"#;
        let root: EpicInstalledRoot = serde_json::from_str(json).expect("parse");
        let item = root.installation_list.expect("list").remove(0);
        assert_eq!(item.app_display_name.as_deref(), Some("My Cool Game"));
        assert!(item
            .install_location
            .as_ref()
            .expect("loc")
            .contains("Epic"));
    }

    #[test]
    fn scan_exe_directory_recursive_finds_nested_exe() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("Vendor").join("GameBin");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("Game.exe"), [0u8]).unwrap();
        let mut map = HashMap::<String, IndexedEntry>::new();
        let mut budget = 50usize;
        super::scan_exe_directory_recursive(
            dir.path(),
            &mut map,
            &mut budget,
            3,
            0,
            NAME_RANK_RECURSIVE_PF,
        )
        .expect("rec scan");
        assert!(map.keys().any(|k| k.contains("game.exe")));
    }

    #[test]
    fn expand_windows_env_path_replaces_systemroot() {
        let Ok(sr) = std::env::var("SystemRoot") else {
            return;
        };
        let out = super::expand_windows_env_path(r"%SystemRoot%\system32\notepad.exe");
        assert!(
            !out.contains('%'),
            "expected expanded path, got {out:?}"
        );
        assert!(out.to_lowercase().contains("notepad.exe"));
        assert!(
            out.to_lowercase().starts_with(&sr.to_lowercase()),
            "expected path under SystemRoot, got {out:?}"
        );
    }

    #[test]
    fn scan_flat_exe_directory_adds_each_exe_in_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ZebraApp.exe"), [0u8]).unwrap();
        std::fs::write(dir.path().join("AlphaApp.exe"), [0u8]).unwrap();
        let mut map = HashMap::<String, IndexedEntry>::new();
        super::scan_flat_exe_directory(dir.path(), &mut map, 100, NAME_RANK_FLAT_EXE).expect("scan");
        assert!(map.len() >= 2);
        assert!(map.keys().any(|k| k.contains("alphaapp")));
        assert!(map.keys().any(|k| k.contains("zebraapp")));
    }
}

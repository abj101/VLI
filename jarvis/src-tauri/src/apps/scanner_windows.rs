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
use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{HWND, MAX_PATH};
use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink, SLGP_RAWPATH};
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

    // Registry passes (no COM required)
    scan_uninstall_registry(&mut map);
    scan_app_paths_registry(&mut map);

    // COM-dependent passes
    let _com = ComApartment::new()?;
    scan_start_menu(&mut map)?;
    scan_get_start_apps(&mut map);

    // Game launchers
    scan_steam(&mut map);
    scan_epic(&mut map);
    scan_gog(&mut map);

    // UWP / Microsoft Store
    scan_uwp(&mut map);

    // Windows built-in accessories
    seed_windows_accessories(&mut map);

    // Recursive exe scan (Program Files, depth ≤ 3); never overwrites richer entries
    scan_program_files_recursive(&mut map);

    Ok(map.into_values().map(|e| e.inner).collect())
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
            let icon = extract_icon_data_url(&exe_path);
            insert_entry(
                map,
                exe_path,
                display_name,
                icon,
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

fn install_dir_skipped_stem(stem: &str) -> bool {
    const SKIP: &[&str] = &[
        "uninstall",
        "unins000",
        "unins001",
        "unins002",
        "setup",
        "install",
        "update",
        "updater",
        "maintenanceservice",
        "crashreporter",
        "elevate",
        "vc_redist",
        "dxsetup",
        "vcredist",
    ];
    let s = stem.to_lowercase();
    SKIP.iter().any(|p| s.contains(p))
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
            visit_dir_lnk(&root, &shell_link, &persist, map)?;
        }
    }
    Ok(())
}

fn visit_dir_lnk(
    dir: &Path,
    shell_link: &IShellLinkW,
    persist: &IPersistFile,
    map: &mut HashMap<String, MapEntry>,
) -> Result<(), String> {
    for e in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
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
                // Accept both .exe targets and protocol-style targets (uwp:)
                if !target.contains("://") && !target_path.exists() {
                    continue;
                }
                let label = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("App")
                    .to_string();
                let icon = if target_path.exists() {
                    extract_icon_data_url(&target)
                } else {
                    None
                };
                insert_entry(
                    map,
                    target,
                    label,
                    icon,
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
            // Find the primary exe in the game directory
            if let Some(exe) = install_dir_primary_exe(&game_dir.to_string_lossy(), &name) {
                if Path::new(&exe).exists() {
                    let icon = extract_icon_data_url(&exe);
                    insert_entry(map, exe, name, icon, SourcePriority::Steam);
                }
            } else {
                // Recurse one level deeper for games with subdirectory launchers
                if let Ok(sub_entries) = std::fs::read_dir(&game_dir) {
                    for sub in sub_entries.filter_map(|e| e.ok()) {
                        if sub.path().is_dir() {
                            if let Some(exe) =
                                install_dir_primary_exe(&sub.path().to_string_lossy(), &name)
                            {
                                if Path::new(&exe).exists() {
                                    let icon = extract_icon_data_url(&exe);
                                    insert_entry(map, exe, name, icon, SourcePriority::Steam);
                                    break;
                                }
                            }
                        }
                    }
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

        if let Some(exe) = install_dir_primary_exe(&loc, &display_name) {
            if Path::new(&exe).exists() {
                let icon = extract_icon_data_url(&exe);
                insert_entry(map, exe, display_name, icon, SourcePriority::Epic);
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

            let icon = extract_icon_data_url(&exe);
            insert_entry(map, exe, name, icon, SourcePriority::Gog);
        }
    }
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
            let icon = extract_icon_data_url(&exe_norm);
            insert_entry(
                map,
                exe_norm,
                (*name).to_string(),
                icon,
                SourcePriority::Accessory,
            );
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// BUG FIX #3: Recursive Program Files scan (depth ≤ 3)
// Previously was flat (depth 1 only), missing all games in subdirectories.
// This pass runs last with lowest priority — never overwrites richer entries.
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
        scan_exe_recursive(&root, map, 0, 6);
    }
}

fn scan_exe_recursive(dir: &Path, map: &mut HashMap<String, MapEntry>, depth: usize, max_depth: usize) {
    if depth > max_depth {
        return;
    }
    if !dir.is_dir() {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.is_dir() {
            scan_exe_recursive(&p, map, depth + 1, max_depth);
        } else if p.is_file() {
            let is_exe = p
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("exe"))
                .unwrap_or(false);
            if !is_exe {
                continue;
            }
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if install_dir_skipped_stem(stem) {
                continue;
            }

            let exe_norm = std::fs::canonicalize(&p)
                .unwrap_or_else(|_| p.to_path_buf())
                .to_string_lossy()
                .to_string();
            // Only add if not already discovered by a richer source
            let key = exe_norm.to_lowercase();
            if map.contains_key(&key) {
                continue;
            }

            let label = display_name_from_app_paths_key(stem);
            insert_entry(
                map,
                exe_norm,
                label,
                None,
                SourcePriority::ExeScan,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Icon extraction via PowerShell (unchanged — runs async-style per entry)
// ---------------------------------------------------------------------------
fn extract_icon_data_url(exe_path: &str) -> Option<String> {
    // Only extract icons for real filesystem paths, not UWP URIs
    if !Path::new(exe_path).exists() {
        return None;
    }
    let escaped = exe_path.replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop';\
         Add-Type -AssemblyName System.Drawing;\
         $icon=[System.Drawing.Icon]::ExtractAssociatedIcon('{escaped}');\
         if($null -eq $icon){{exit 0}};\
         $bmp=$icon.ToBitmap();\
         $ms=New-Object System.IO.MemoryStream;\
         $bmp.Save($ms,[System.Drawing.Imaging.ImageFormat]::Png);\
         [Convert]::ToBase64String($ms.ToArray())"
    );
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let b64 = String::from_utf8(out.stdout).ok()?;
    let t = b64.trim();
    if t.is_empty() {
        return None;
    }
    Some(format!("data:image/png;base64,{t}"))
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

    #[test]
    fn display_name_from_app_paths_key_title_cases_stem() {
        assert_eq!(display_name_from_app_paths_key("firefox.exe"), "Firefox");
        assert_eq!(display_name_from_app_paths_key("code.exe"), "Code");
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

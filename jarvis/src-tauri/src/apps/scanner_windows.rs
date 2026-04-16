//! Registry Uninstall + Start Menu `.lnk` crawl (Windows).

use super::AppEntry;
use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::MAX_PATH;
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
    let mut map: HashMap<String, AppEntry> = HashMap::new();
    // Fast registry pass: fills most launchers without shell COM (icons added later by merge).
    scan_app_paths_registry(&mut map)?;
    scan_uninstall_registry(&mut map)?;
    let _com = ComApartment::new()?;
    scan_start_menu(&mut map)?;
    seed_windows_accessories(&mut map);
    Ok(map.into_values().collect())
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
        let (name, val) = res.ok()?;
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

fn seed_windows_accessories(map: &mut HashMap<String, AppEntry>) {
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
        insert_entry(map, exe_norm, (*name).into(), None);
    }
}

/// `App Paths` maps `something.exe` → default value full path (broad coverage vs Uninstall alone).
fn scan_app_paths_registry(map: &mut HashMap<String, AppEntry>) -> Result<(), String> {
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
            insert_entry(map, exe_norm, display_name, None);
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

fn scan_uninstall_registry(map: &mut HashMap<String, AppEntry>) -> Result<(), String> {
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
            insert_entry(map, exe_norm, display_name, icon_data_url);
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
    let mut best: Option<(usize, PathBuf)> = None;
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
        if score == 0 {
            continue;
        }
        let replace = match &best {
            None => true,
            Some((s, _)) => score > *s,
        };
        if replace {
            best = Some((score, p.clone()));
        }
    }
    if let Some((_, p)) = best {
        return Some(p.to_string_lossy().to_string());
    }
    if exes.len() == 1 {
        return Some(exes[0].to_string_lossy().to_string());
    }
    None
}

fn scan_start_menu(map: &mut HashMap<String, AppEntry>) -> Result<(), String> {
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| e.to_string())?;
    let persist: IPersistFile = shell_link.cast().map_err(|e| e.to_string())?;

    let mut roots = Vec::new();
    if let Ok(pd) = std::env::var("PROGRAMDATA") {
        roots.push(PathBuf::from(pd).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Ok(ad) = std::env::var("APPDATA") {
        roots.push(PathBuf::from(ad).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(la).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Ok(public) = std::env::var("PUBLIC") {
        roots.push(PathBuf::from(public).join("Desktop"));
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
    map: &mut HashMap<String, AppEntry>,
) -> Result<(), String> {
    let read = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for e in read {
        let e = e.map_err(|e| e.to_string())?;
        let p = e.path();
        if p.is_dir() {
            visit_dir_lnk(&p, shell_link, persist, map)?;
        } else if p.extension().and_then(|x| x.to_str()) == Some("lnk") {
            if let Some(target) = resolve_lnk(shell_link, persist, &p) {
                if target.is_empty() || !Path::new(&target).exists() {
                    continue;
                }
                let label = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("App")
                    .to_string();
                let icon_data_url = extract_icon_data_url(&target);
                insert_entry(map, target, label, icon_data_url);
            }
        }
    }
    Ok(())
}

fn resolve_lnk(shell_link: &IShellLinkW, persist: &IPersistFile, lnk: &Path) -> Option<String> {
    let wide: Vec<u16> = lnk
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
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
    map: &mut HashMap<String, AppEntry>,
    exe_path: String,
    display_name: String,
    icon_data_url: Option<String>,
) {
    let key = exe_path.to_lowercase();
    let entry = AppEntry {
        display_name,
        exe_path,
        icon_data_url,
    };
    map.entry(key)
        .and_modify(|existing| {
            if entry.display_name.len() > existing.display_name.len() {
                *existing = entry.clone();
            } else if existing.icon_data_url.is_none() && entry.icon_data_url.is_some() {
                existing.icon_data_url = entry.icon_data_url.clone();
            }
        })
        .or_insert(entry);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_entry_backfills_missing_icon_on_existing_row() {
        let mut map = HashMap::<String, AppEntry>::new();
        insert_entry(
            &mut map,
            r"C:\Apps\Example.exe".into(),
            "Example".into(),
            None,
        );
        insert_entry(
            &mut map,
            r"C:\Apps\Example.exe".into(),
            "Example".into(),
            Some("data:image/png;base64,AAA=".into()),
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
    fn display_name_from_app_paths_key_title_cases_stem() {
        assert_eq!(display_name_from_app_paths_key("firefox.exe"), "Firefox");
        assert_eq!(display_name_from_app_paths_key("WINWORD.EXE"), "WINWORD");
        assert_eq!(display_name_from_app_paths_key("code.exe"), "Code");
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
}

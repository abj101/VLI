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
use winreg::RegKey;

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
    let _com = ComApartment::new()?;
    let mut map: HashMap<String, AppEntry> = HashMap::new();
    scan_uninstall_registry(&mut map)?;
    scan_start_menu(&mut map)?;
    Ok(map.into_values().collect())
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
            let exe = sub
                .get_value::<String, _>("DisplayIcon")
                .ok()
                .and_then(|s| display_icon_to_exe(&s))
                .or_else(|| {
                    sub.get_value::<String, _>("InstallLocation")
                        .ok()
                        .and_then(|loc| install_location_guess(&display_name, &loc))
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
    let trimmed = raw.trim().trim_matches('"');
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
}

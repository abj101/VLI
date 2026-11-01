use crate::apps::AppEntry;
use crate::window::FocusedWindow;

/// Build a short, request-only description of the app that had focus when listening began.
pub fn focus_context(focus: &FocusedWindow, apps: &[AppEntry]) -> String {
    let exe = exe_basename(&focus.exe_path);
    let app_name = apps
        .iter()
        .find(|app| exe_basename(&app.exe_path).eq_ignore_ascii_case(&exe))
        .map(|app| app.display_name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| exe_stem(&exe));
    format!("the user is in {app_name}")
}

fn exe_basename(path: &str) -> String {
    path.trim()
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(path.trim())
        .to_string()
}

fn exe_stem(exe: &str) -> String {
    exe.rsplit_once('.')
        .map(|(stem, _)| stem)
        .filter(|stem| !stem.is_empty())
        .unwrap_or(exe)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focus(exe_path: &str) -> FocusedWindow {
        FocusedWindow {
            exe_path: exe_path.to_string(),
        }
    }

    fn app(display_name: &str, exe_path: &str) -> AppEntry {
        AppEntry {
            display_name: display_name.to_string(),
            exe_path: exe_path.to_string(),
            icon_data_url: None,
        }
    }

    #[test]
    fn index_hit_matches_executable_basename_case_insensitively() {
        let apps = [app("Google Chrome", r"C:\Program Files\Chrome\chrome.exe")];
        assert_eq!(
            focus_context(&focus(r"D:\Apps\CHROME.EXE"), &apps),
            "the user is in Google Chrome"
        );
    }

    #[test]
    fn index_miss_uses_executable_stem() {
        assert_eq!(
            focus_context(&focus("spotify.exe"), &[]),
            "the user is in spotify"
        );
    }
}

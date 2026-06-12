//! Runtime execution for smart `open_target` resolution + launch.

use crate::{
    apps::{
        resolve_target::{
            ambiguous_to_choice, interpret_clarify_choice, resolve_target, strip_placement_suffix,
            ResolvedTarget,
        },
        AppEntry,
    },
    commands::executor::ActionRuntime,
    db::{TargetAlias, TargetAliasKind},
    window::{
        focus_existing_app_window, place_window_after_app_launch, placement_status_label,
        snap_foreground_window, snapshot_top_level_windows, DEFAULT_MONITOR,
    },
};
use rusqlite::Connection;
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

const CLARIFY_PROMPT: &str = "Open as an app or in the browser?";

pub fn execute_open_target(
    conn: &Connection,
    target: &str,
    placement: Option<&str>,
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
) -> Result<(), String> {
    let aliases = crate::db::get_all_target_aliases(conn).map_err(|e| e.to_string())?;
    let resolved = resolve_open_target_input(target, placement, app_index.unwrap_or(&[]), &aliases);
    let final_target = match resolved {
        ResolvedTarget::Ambiguous { .. } => {
            clarify_and_resolve(&resolved, runtime, app_index.unwrap_or(&[]))?
        }
        other => other,
    };
    runtime.emit_status(&status_message_for(&final_target));
    launch_resolved(&final_target, runtime, app_index)
}

pub fn execute_open_target_tool(
    conn: &Connection,
    args: &HashMap<String, String>,
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
) -> Result<(), String> {
    let target = args
        .get("target")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "missing required tool argument `target` for `open_target`".to_string())?;
    let placement = args
        .get("placement")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    execute_open_target(conn, target, placement, runtime, app_index)
}

pub fn execute_open_target_with_aliases(
    target: &str,
    placement: Option<&str>,
    aliases: &[TargetAlias],
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
) -> Result<String, String> {
    let entries = app_index.unwrap_or(&[]);
    let resolved = resolve_open_target_input(target, placement, entries, aliases);
    let final_target = match &resolved {
        ResolvedTarget::Ambiguous { .. } => clarify_and_resolve(&resolved, runtime, entries)?,
        _ => resolved,
    };
    let status = status_message_for(&final_target);
    launch_resolved(&final_target, runtime, app_index)?;
    Ok(status.replace('…', "..."))
}

fn resolve_open_target_input(
    target: &str,
    placement: Option<&str>,
    app_entries: &[AppEntry],
    aliases: &[TargetAlias],
) -> ResolvedTarget {
    let explicit_placement = placement
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let (clean_target, suffix_placement) = strip_placement_suffix(target);
    let zone = explicit_placement.or(suffix_placement);
    let mut resolved = resolve_target(&clean_target, app_entries, aliases);
    if let Some(zone) = zone {
        attach_placement(&mut resolved, zone);
    }
    resolved
}

fn attach_placement(resolved: &mut ResolvedTarget, placement: String) {
    match resolved {
        ResolvedTarget::App { placement: slot, .. } => *slot = Some(placement),
        ResolvedTarget::Url { placement: slot, .. } => *slot = Some(placement),
        ResolvedTarget::Ambiguous { placement: slot, .. } => *slot = Some(placement),
    }
}

fn clarify_and_resolve(
    ambiguous: &ResolvedTarget,
    runtime: &impl ActionRuntime,
    _app_entries: &[AppEntry],
) -> Result<ResolvedTarget, String> {
    if let Err(err) = runtime.speak(CLARIFY_PROMPT) {
        runtime.emit_status(&format!(
            "Follow-up prompt voice unavailable ({err}); showing text prompt"
        ));
    }
    let response = runtime.request_follow_up(CLARIFY_PROMPT)?;
    runtime.emit_status(&response);
    let choose_app = interpret_clarify_choice(&response, ambiguous).unwrap_or_else(|| {
        // Default: prefer URL when an app candidate is below threshold.
        let app_weak = match ambiguous {
            ResolvedTarget::Ambiguous { app_candidates, .. } => {
                app_candidates.first().map(|c| c.score).unwrap_or(0.0)
                    < crate::apps::APP_RESOLVE_MIN_RATIO
            }
            _ => true,
        };
        !app_weak
    });
    let resolved = ambiguous_to_choice(ambiguous, choose_app).ok_or_else(|| {
        format!("Could not interpret follow-up for `{response}`")
    })?;
    learn_alias_from_clarify(runtime, ambiguous, choose_app);
    Ok(resolved)
}

fn learn_alias_from_clarify(
    runtime: &impl ActionRuntime,
    ambiguous: &ResolvedTarget,
    choose_app: bool,
) {
    let ResolvedTarget::Ambiguous {
        query,
        app_candidates,
        url_candidate,
        ..
    } = ambiguous
    else {
        return;
    };
    let spoken = query.trim().to_lowercase();
    if spoken.is_empty() {
        return;
    }
    let alias = if choose_app {
        let Some(top) = app_candidates.first() else {
            return;
        };
        TargetAlias {
            spoken: spoken.clone(),
            kind: TargetAliasKind::App,
            value: top.exe_path.clone(),
        }
    } else {
        TargetAlias {
            spoken: spoken.clone(),
            kind: TargetAliasKind::Url,
            value: if url_candidate.url.is_empty() {
                format!("https://{spoken}.com")
            } else {
                url_candidate.url.clone()
            },
        }
    };
    if let Err(err) = runtime.persist_target_alias(&alias) {
        runtime.emit_error(&format!("Could not remember alias: {err}"));
    } else {
        runtime.emit_status(&format!(
            "Remembered `{spoken}` as {}",
            alias_kind_label(&alias)
        ));
    }
}

fn alias_kind_label(alias: &TargetAlias) -> &'static str {
    match alias.kind {
        TargetAliasKind::App => "app",
        TargetAliasKind::Url => "browser",
    }
}

fn launch_resolved(
    resolved: &ResolvedTarget,
    runtime: &impl ActionRuntime,
    app_index: Option<&[AppEntry]>,
) -> Result<(), String> {
    match resolved {
        ResolvedTarget::App {
            display_name,
            exe_path,
            placement,
        } => {
            let path = resolve_app_launch_path(display_name, exe_path, app_index)?;
            let zone = placement.as_deref().filter(|z| !z.trim().is_empty());
            if focus_existing_app_window(&path, display_name, zone, DEFAULT_MONITOR)? {
                runtime.emit_status(&format!(
                    "Focused {display_name}{}",
                    zone.map(|z| format!(" ({})", placement_status_label(z)))
                        .unwrap_or_default()
                ));
                return Ok(());
            }
            let snapshot = zone.map(|_| snapshot_top_level_windows());
            runtime.open_app(&path)?;
            if let (Some(zone), Some(before)) = (zone, snapshot) {
                apply_placement_after_launch(runtime, &path, zone, before)?;
            }
            Ok(())
        }
        ResolvedTarget::Url { url, placement } => {
            validate_https_url(url)?;
            runtime.open_url(url)?;
            if let Some(zone) = placement.as_deref().filter(|z| !z.trim().is_empty()) {
                thread::sleep(Duration::from_millis(600));
                snap_foreground_window(zone, DEFAULT_MONITOR).inspect_err(|err| {
                    runtime.emit_error(err);
                })?;
                runtime.emit_status(&format!(
                    "Placed browser window ({})",
                    placement_status_label(zone)
                ));
            }
            Ok(())
        }
        ResolvedTarget::Ambiguous { query, .. } => Err(format!(
            "target `{query}` is still ambiguous after clarification"
        )),
    }
}

fn apply_placement_after_launch(
    runtime: &impl ActionRuntime,
    exe_path: &str,
    zone: &str,
    before: crate::window::WindowSnapshot,
) -> Result<(), String> {
    match place_window_after_app_launch(exe_path, zone, DEFAULT_MONITOR, &before) {
        Ok(()) => {
            runtime.emit_status(&format!(
                "Placed window ({})",
                placement_status_label(zone)
            ));
            Ok(())
        }
        Err(err) => {
            runtime.emit_error(&err);
            Err(err)
        }
    }
}

fn resolve_app_launch_path(
    display_name: &str,
    exe_path: &str,
    app_index: Option<&[AppEntry]>,
) -> Result<String, String> {
    let trimmed = exe_path.trim();
    if trimmed.contains('\\') || trimmed.contains('/') || trimmed.ends_with(".exe") {
        return Ok(trimmed.to_string());
    }
    if let Some(entries) = app_index {
        if let Some(hit) = crate::apps::resolve_app(display_name, entries) {
            return Ok(hit.exe_path.clone());
        }
        if let Some(hit) = crate::apps::resolve_app(trimmed, entries) {
            return Ok(hit.exe_path.clone());
        }
    }
    Ok(trimmed.to_string())
}

fn validate_https_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(())
    } else {
        Err(format!(
            "OpenUrl only supports http:// or https:// URLs: `{trimmed}`"
        ))
    }
}

fn status_message_for(resolved: &ResolvedTarget) -> String {
    match resolved {
        ResolvedTarget::App {
            display_name,
            placement,
            ..
        } => format_opening_status(display_name, placement.as_deref()),
        ResolvedTarget::Url { url, placement } => {
            format_opening_status(url, placement.as_deref())
        }
        ResolvedTarget::Ambiguous { query, .. } => format!("Ambiguous target `{query}`"),
    }
}

fn format_opening_status(label: &str, placement: Option<&str>) -> String {
    match placement.filter(|z| !z.trim().is_empty()) {
        Some(zone) => format!(
            "Opening {} ({})…",
            label,
            placement_status_label(zone)
        ),
        None => format!("Opening {label}…"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::AppEntry;
    use crate::db::{TargetAlias, TargetAliasKind};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct MockState {
        app_calls: Vec<String>,
        url_calls: Vec<String>,
        statuses: Vec<String>,
        errors: Vec<String>,
        follow_up_answers: Vec<String>,
        follow_up_prompts: Vec<String>,
        learned_aliases: Vec<TargetAlias>,
    }

    #[derive(Clone, Default)]
    struct MockRuntime {
        state: Arc<Mutex<MockState>>,
    }

    impl ActionRuntime for MockRuntime {
        fn open_app(&self, path: &str) -> Result<(), String> {
            self.state.lock().unwrap().app_calls.push(path.to_string());
            Ok(())
        }

        fn open_url(&self, url: &str) -> Result<(), String> {
            self.state.lock().unwrap().url_calls.push(url.to_string());
            Ok(())
        }

        fn run_script(&self, _script: &str, _args: &[String]) -> Result<(), String> {
            Ok(())
        }

        fn send_keys(&self, _keys: &str) -> Result<(), String> {
            Ok(())
        }

        fn wait_ms(&self, _ms: u64) -> Result<(), String> {
            Ok(())
        }

        fn speak(&self, _text: &str) -> Result<(), String> {
            Ok(())
        }

        fn request_follow_up(&self, prompt: &str) -> Result<String, String> {
            let mut s = self.state.lock().unwrap();
            s.follow_up_prompts.push(prompt.to_string());
            if s.follow_up_answers.is_empty() {
                return Err("no follow-up".into());
            }
            Ok(s.follow_up_answers.remove(0))
        }

        fn is_cancelled(&self) -> bool {
            false
        }

        fn emit_status(&self, text: &str) {
            self.state.lock().unwrap().statuses.push(text.to_string());
        }

        fn emit_error(&self, message: &str) {
            self.state.lock().unwrap().errors.push(message.to_string());
        }

        fn persist_target_alias(&self, alias: &TargetAlias) -> Result<(), String> {
            self.state.lock().unwrap().learned_aliases.push(alias.clone());
            Ok(())
        }
    }

    fn brave_entry() -> AppEntry {
        AppEntry {
            display_name: "Brave".into(),
            exe_path: r"C:\JarvisTestOnly\no-real-brave-xyz.exe".into(),
            icon_data_url: None,
        }
    }

    #[test]
    fn execute_open_target_launches_resolved_app() {
        let runtime = MockRuntime::default();
        let index = vec![brave_entry()];
        let msg = execute_open_target_with_aliases("brave", None, &[], &runtime, Some(&index))
            .expect("open brave");
        assert!(msg.contains("Brave"));
        assert_eq!(
            runtime.state.lock().unwrap().app_calls,
            vec![r"C:\JarvisTestOnly\no-real-brave-xyz.exe".to_string()]
        );
    }

    #[test]
    fn execute_open_target_alias_skips_clarify() {
        let runtime = MockRuntime::default();
        let index = vec![AppEntry {
            display_name: "GitHub Desktop".into(),
            exe_path: "gh.exe".into(),
            icon_data_url: None,
        }];
        let aliases = vec![TargetAlias {
            spoken: "github".into(),
            kind: TargetAliasKind::Url,
            value: "https://github.com".into(),
        }];
        execute_open_target_with_aliases("github", None, &aliases, &runtime, Some(&index))
            .expect("alias url");
        assert_eq!(
            runtime.state.lock().unwrap().url_calls,
            vec!["https://github.com".to_string()]
        );
    }

    #[test]
    fn clarify_learns_url_alias() {
        let runtime = MockRuntime {
            state: Arc::new(Mutex::new(MockState {
                follow_up_answers: vec!["browser".into()],
                ..Default::default()
            })),
        };
        let index = vec![AppEntry {
            display_name: "GitHub".into(),
            exe_path: "gh.exe".into(),
            icon_data_url: None,
        }];
        execute_open_target_with_aliases("github", None, &[], &runtime, Some(&index))
            .expect("clarified");
        let learned = &runtime.state.lock().unwrap().learned_aliases;
        assert_eq!(learned.len(), 1);
        assert_eq!(learned[0].spoken, "github");
        assert_eq!(learned[0].kind, TargetAliasKind::Url);
        assert_eq!(learned[0].value, "https://github.com");
    }

    #[test]
    fn execute_open_target_ambiguous_uses_follow_up() {
        let runtime = MockRuntime {
            state: Arc::new(Mutex::new(MockState {
                follow_up_answers: vec!["browser".into()],
                ..Default::default()
            })),
        };
        let index = vec![AppEntry {
            display_name: "GitHub".into(),
            exe_path: "gh.exe".into(),
            icon_data_url: None,
        }];
        execute_open_target_with_aliases("github", None, &[], &runtime, Some(&index))
            .expect("clarified url");
        assert_eq!(
            runtime.state.lock().unwrap().url_calls,
            vec!["https://github.com".to_string()]
        );
        assert_eq!(
            runtime.state.lock().unwrap().follow_up_prompts,
            vec![CLARIFY_PROMPT.to_string()]
        );
    }
}

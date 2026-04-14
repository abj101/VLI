mod apps;
mod audio;
mod commands;
mod db;
mod hud;
mod keychain;
mod tray;

use audio::SharedAudioPipeline;
use commands::TauriActionRuntime;
use hud::{sync_hud_window, HudPhase, HUD_WINDOW_LABEL};
use log::{debug, info, warn};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Listener, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{Builder as ShortcutBuilder, GlobalShortcutExt, ShortcutState};

const AUTO_DISMISS_AFTER: Duration = Duration::from_secs(4);
const NO_MATCH_TIMEOUT: Duration = Duration::from_secs(5);
const EDITOR_WINDOW_LABEL: &str = "editor";
const DEFAULT_HOTKEY: &str = "ctrl+shift+j";
const SETTING_KEY_HOTKEY: &str = "hotkey";
const SETTING_KEY_DEFAULT_THRESHOLD: &str = "default_fuzzy_threshold_pct";
const DEFAULT_THRESHOLD_PCT: u16 = 80;
const EDITOR_COMMANDS_CHANGED_EVENT: &str = "editor-commands-changed";
const APP_INDEX_READY_EVENT: &str = "app-index-ready";
pub(crate) const OPEN_SETTINGS_EVENT: &str = "open-settings";
/// After the last speech-related activity, wait this long before treating speech as finished
/// (starts the 4s auto-dismiss countdown only after this gap).
const SILENCE_BEFORE_AUTO_DISMISS: Duration = Duration::from_millis(450);
/// Debounce partial STT updates so commands do not fire mid-sentence.
const SILENCE_BEFORE_MATCH: Duration = Duration::from_millis(550);
/// Amplitude above this (0..1) counts as speech for activity / silence detection.
const SPEECH_AMPLITUDE_THRESHOLD: f64 = 0.02;
const FOLLOW_UP_TIMEOUT: Duration = Duration::from_secs(8);
const FOLLOW_UP_TIMEOUT_MSG: &str = "Follow-up input timed out";
const ACTION_RUN_CANCELLED_MSG: &str = "Action run cancelled";

type ActionPayload = db::Action;
type CommandCache = Arc<RwLock<Vec<db::CommandNode>>>;
/// Cached installed-app entries for `OpenApp` resolution (path optional).
type AppIndexStore = Arc<RwLock<Vec<apps::AppEntry>>>;

const APP_INDEX_CACHE_MAX_AGE_SECS: u64 = 24 * 60 * 60;

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn refresh_app_index_on_startup(app: &AppHandle, store: &AppIndexStore) -> Result<(), String> {
    let conn = open_db_connection(app)?;
    let entries = db::load_app_index(&conn).map_err(|e| e.to_string())?;
    let last = db::get_app_index_last_scan_unix(&conn).map_err(|e| e.to_string())?;
    let now = now_unix_secs();
    let stale = match last {
        None => true,
        Some(t) => now.saturating_sub(t) > APP_INDEX_CACHE_MAX_AGE_SECS as i64,
    };
    {
        let mut g = store
            .write()
            .map_err(|_| "app index lock poisoned".to_string())?;
        *g = entries;
    }
    let count = store
        .read()
        .map_err(|_| "app index lock poisoned".to_string())?
        .len();
    let _ = app.emit(
        APP_INDEX_READY_EVENT,
        serde_json::json!({ "count": count }),
    );
    if stale || count == 0 {
        let app_h = app.clone();
        let st = Arc::clone(store);
        std::thread::spawn(move || {
            let scanned = apps::scan_installed_apps();
            let unix = now_unix_secs();
            if let Ok(conn) = open_db_connection(&app_h) {
                let _ = db::replace_app_index(&conn, &scanned, unix);
            }
            if let Ok(mut g) = st.write() {
                *g = scanned;
            }
            let n = st.read().map(|g| g.len()).unwrap_or(0);
            let _ = app_h.emit(APP_INDEX_READY_EVENT, serde_json::json!({ "count": n }));
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
struct CommandNodePayload {
    name: String,
    trigger_phrases: Vec<String>,
    actions: Vec<ActionPayload>,
    enabled: bool,
    fuzzy_threshold_pct: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReorderCommandsPayload {
    ordered_ids: Vec<i64>,
}

impl CommandNodePayload {
    fn try_into_new_command_node(&self) -> Result<db::NewCommandNode, String> {
        validate_command_node_payload(self)?;
        let name = self.name.trim().to_string();
        let trigger_phrases = self
            .trigger_phrases
            .iter()
            .map(|phrase| phrase.trim())
            .filter(|phrase| !phrase.is_empty())
            .map(ToString::to_string)
            .collect();
        Ok(db::NewCommandNode {
            name,
            trigger_phrases,
            actions: self.actions.clone(),
            enabled: self.enabled,
            fuzzy_threshold_pct: self.fuzzy_threshold_pct as u16,
        })
    }
}

fn validate_command_node_payload(payload: &CommandNodePayload) -> Result<(), String> {
    if payload.name.trim().is_empty() {
        return Err("name is required".to_string());
    }
    let has_any_trigger = payload
        .trigger_phrases
        .iter()
        .any(|phrase| !phrase.trim().is_empty());
    if !has_any_trigger {
        return Err("at least one trigger phrase is required".to_string());
    }
    if !(0..=100).contains(&payload.fuzzy_threshold_pct) {
        return Err("fuzzy threshold must be between 0 and 100".to_string());
    }
    Ok(())
}

fn open_db_connection(app: &AppHandle) -> Result<rusqlite::Connection, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let db_path = dir.join("jarvis.db");
    rusqlite::Connection::open(db_path).map_err(|e| e.to_string())
}

fn validate_setting_key(key: &str) -> Result<String, String> {
    let normalized = key.trim();
    if normalized.is_empty() {
        return Err("setting key is required".to_string());
    }
    Ok(normalized.to_string())
}

fn normalize_hotkey_input(hotkey: &str) -> Result<String, String> {
    let normalized = hotkey.trim();
    if normalized.is_empty() {
        return Err("hotkey is required".to_string());
    }
    Ok(normalized.to_string())
}

fn resolve_fuzzy_threshold_pct(node_threshold_pct: u16, default_threshold_pct: u16) -> u16 {
    if node_threshold_pct == 0 {
        return default_threshold_pct.clamp(50, 100);
    }
    node_threshold_pct.clamp(1, 100)
}

fn load_default_fuzzy_threshold_pct(app: &AppHandle) -> u16 {
    let Ok(conn) = open_db_connection(app) else {
        return DEFAULT_THRESHOLD_PCT;
    };
    let Ok(value_opt) = db::get_setting(&conn, SETTING_KEY_DEFAULT_THRESHOLD) else {
        return DEFAULT_THRESHOLD_PCT;
    };
    let Some(raw) = value_opt else {
        return DEFAULT_THRESHOLD_PCT;
    };
    match raw.trim().parse::<u16>() {
        Ok(value) if (50..=100).contains(&value) => value,
        _ => DEFAULT_THRESHOLD_PCT,
    }
}

#[derive(Debug, Default)]
struct HudRuntime {
    phase: HudPhase,
    visible: bool,
    session_id: u64,
    /// Last time we saw speech (transcript text or mic level). Used so timers run on silence, not wall-clock from HUD open.
    last_speech_activity: Option<Instant>,
    /// Last non-empty transcript seen while listening.
    pending_transcript: String,
    /// Monotonic version for pending transcript debounce scheduling.
    transcript_revision: u64,
    /// Cooperative cancellation handle for currently running action chain.
    active_run_cancel: Option<Arc<AtomicBool>>,
    /// Session id that owns `active_run_cancel`.
    active_run_session_id: Option<u64>,
    /// Final transcript captured while waiting in `AwaitingInput`.
    pending_follow_up_response: Option<String>,
    /// Latest non-empty follow-up candidate captured from streaming STT updates.
    pending_follow_up_candidate: Option<String>,
    /// Last update timestamp for candidate debounce.
    pending_follow_up_candidate_at: Option<Instant>,
}

type SharedHud = Arc<Mutex<HudRuntime>>;

/// Holds the running wake worker so `update_settings` can restart it (live reload).
pub struct WakeSupervisorState(pub Mutex<Option<audio::wake::thread::WakeSupervisor>>);

impl Default for WakeSupervisorState {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}

#[derive(Debug)]
struct HotkeyBindingState {
    current: Mutex<String>,
}

fn preview_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    let head: String = s.chars().take(max).collect();
    if n > max {
        format!("{head}…")
    } else {
        head
    }
}

fn load_stt_pipeline_choice(app: &AppHandle) -> audio::SttPipelineChoice {
    let Ok(conn) = open_db_connection(app) else {
        return audio::SttPipelineChoice::Local;
    };
    let Ok(settings) = db::get_app_settings(&conn) else {
        return audio::SttPipelineChoice::Local;
    };
    match audio::transcription::parse_stt_provider(Some(settings.stt_provider.as_str())) {
        audio::transcription::SttProvider::Local => audio::SttPipelineChoice::Local,
        audio::transcription::SttProvider::Os => audio::SttPipelineChoice::Os,
        audio::transcription::SttProvider::Remote => {
            let key = if settings.remote_stt_key_stored {
                keychain::get_api_key("remote_stt").ok().flatten()
            } else {
                None
            };
            let params = audio::RemoteSttParams {
                endpoint: settings.remote_stt_url.clone(),
                model: settings.remote_stt_model.clone(),
                bearer_token: key,
                timeout: audio::RemoteSttParams::sanitized_timeout(settings.remote_stt_timeout_secs),
            };
            audio::SttPipelineChoice::Remote(params)
        }
    }
}

fn try_start_listening_audio(app: &AppHandle, slot: &SharedAudioPipeline, hud_session_id: u64) {
    let old = {
        let mut g = slot.lock().unwrap();
        g.take()
    };
    drop(old);

    let choice = load_stt_pipeline_choice(app);
    match audio::AudioPipeline::start(app, hud_session_id, choice) {
        Ok(p) => {
            let mut g = slot.lock().unwrap();
            *g = Some(p);
        }
        Err(msg) => {
            let _ = app.emit("audio-error", serde_json::json!({ "message": msg }));
        }
    }
}

fn emit_hud_phase(app: &AppHandle, phase: HudPhase) {
    let _ = app.emit("hud-phase", serde_json::json!({ "phase": phase.as_str() }));
}

fn load_all_commands(app: &AppHandle) -> Result<Vec<db::CommandNode>, String> {
    let conn = open_db_connection(app)?;
    db::get_all_commands(&conn).map_err(|e| e.to_string())
}

fn read_command_cache(cache: &CommandCache) -> Result<Vec<db::CommandNode>, String> {
    cache
        .read()
        .map_err(|_| "command cache lock poisoned".to_string())
        .map(|rows| rows.clone())
}

fn refresh_command_cache_from_rows(
    cache: &CommandCache,
    rows: Vec<db::CommandNode>,
) -> Result<(), String> {
    let mut guard = cache
        .write()
        .map_err(|_| "command cache lock poisoned".to_string())?;
    *guard = rows;
    Ok(())
}

fn refresh_command_cache(
    app: &AppHandle,
    cache: &CommandCache,
) -> Result<Vec<db::CommandNode>, String> {
    let rows = load_all_commands(app)?;
    refresh_command_cache_from_rows(cache, rows.clone())?;
    Ok(rows)
}

fn emit_editor_commands_changed(app: &AppHandle) {
    let _ = app.emit(EDITOR_COMMANDS_CHANGED_EVENT, serde_json::json!({}));
}

fn should_focus_existing_editor_window(window_exists: bool) -> bool {
    window_exists
}

pub(crate) fn open_or_create_editor_window(app: &AppHandle) -> Result<(), String> {
    if should_focus_existing_editor_window(app.get_webview_window(EDITOR_WINDOW_LABEL).is_some()) {
        if let Some(window) = app.get_webview_window(EDITOR_WINDOW_LABEL) {
            window.show().map_err(|e| e.to_string())?;
            window.set_focus().map_err(|e| e.to_string())?;
            return Ok(());
        }
    }

    let window = WebviewWindowBuilder::new(
        app,
        EDITOR_WINDOW_LABEL,
        WebviewUrl::App("editor.html".into()),
    )
    .title("JARVIS Editor")
    .decorations(true)
    .resizable(true)
    .center()
    .min_inner_size(900.0, 600.0)
    .build()
    .map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

/// Live STT emits partial `transcript-update` (`is_final: false`). Matching is gated on a short
/// silence window so commands do not fire before the user finishes their phrase.
fn should_attempt_command_match(rt: &HudRuntime) -> bool {
    rt.visible && rt.phase == HudPhase::Listening
}

fn should_attempt_match_for_update(rt: &HudRuntime, is_final: bool) -> bool {
    if is_final {
        return true;
    }
    match rt.last_speech_activity {
        Some(last) => last.elapsed() >= SILENCE_BEFORE_MATCH,
        None => true,
    }
}

fn update_pending_transcript(rt: &SharedHud, text: &str) -> Option<(u64, u64)> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut s = rt.lock().ok()?;
    if !should_attempt_command_match(&s) {
        return None;
    }
    s.pending_transcript = trimmed.to_string();
    s.transcript_revision = s.transcript_revision.wrapping_add(1);
    s.pending_follow_up_response = None;
    s.pending_follow_up_candidate = None;
    s.pending_follow_up_candidate_at = None;
    Some((s.session_id, s.transcript_revision))
}

fn cancel_active_run_in_state(s: &mut HudRuntime) {
    if let Some(cancel) = s.active_run_cancel.take() {
        cancel.store(true, Ordering::Relaxed);
    }
    s.active_run_session_id = None;
}

fn prepare_hud_listening_session(s: &mut HudRuntime) -> u64 {
    s.visible = true;
    s.phase = HudPhase::Listening;
    s.session_id = s.session_id.wrapping_add(1);
    s.last_speech_activity = Some(Instant::now());
    s.pending_transcript.clear();
    s.transcript_revision = 0;
    s.pending_follow_up_response = None;
    s.pending_follow_up_candidate = None;
    s.pending_follow_up_candidate_at = None;
    cancel_active_run_in_state(s);
    s.session_id
}

fn prepare_hud_close_session(s: &mut HudRuntime) {
    s.phase = HudPhase::Stopped;
    s.visible = false;
    s.session_id = s.session_id.wrapping_add(1);
    s.pending_transcript.clear();
    s.transcript_revision = 0;
    s.pending_follow_up_response = None;
    s.pending_follow_up_candidate = None;
    s.pending_follow_up_candidate_at = None;
    cancel_active_run_in_state(s);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FollowUpAbortReason {
    Cancelled,
    TimedOut,
}

fn take_follow_up_response(rt: &mut HudRuntime, expected_session_id: u64) -> Option<String> {
    if rt.visible && rt.phase == HudPhase::AwaitingInput && rt.session_id == expected_session_id {
        return rt.pending_follow_up_response.take();
    }
    None
}

fn maybe_promote_follow_up_candidate(rt: &mut HudRuntime, now: Instant) {
    if rt.pending_follow_up_response.is_some() {
        return;
    }
    let Some(updated_at) = rt.pending_follow_up_candidate_at else {
        return;
    };
    if now.duration_since(updated_at) < SILENCE_BEFORE_MATCH {
        return;
    }
    if let Some(candidate) = rt.pending_follow_up_candidate.take() {
        rt.pending_follow_up_response = Some(candidate);
        rt.pending_follow_up_candidate_at = None;
    }
}

fn should_abort_follow_up_wait(
    rt: &HudRuntime,
    expected_session_id: u64,
    is_cancelled: bool,
    now: Instant,
    deadline: Instant,
) -> Option<FollowUpAbortReason> {
    if is_cancelled
        || !rt.visible
        || rt.session_id != expected_session_id
        || rt.phase == HudPhase::Stopped
    {
        return Some(FollowUpAbortReason::Cancelled);
    }
    if now >= deadline {
        return Some(FollowUpAbortReason::TimedOut);
    }
    None
}

fn capture_follow_up_from_update(rt: &SharedHud, update: &audio::stt::TranscriptUpdate) -> bool {
    let trimmed = update.text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let mut s = match rt.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    if !s.visible || s.phase != HudPhase::AwaitingInput || s.session_id != update.hud_session_id {
        return false;
    }
    s.pending_transcript = trimmed.to_string();
    s.pending_follow_up_candidate = Some(trimmed.to_string());
    s.pending_follow_up_candidate_at = Some(Instant::now());
    if update.is_final {
        s.pending_follow_up_response = Some(trimmed.to_string());
        s.pending_follow_up_candidate = None;
        s.pending_follow_up_candidate_at = None;
    }
    true
}

fn should_finalize_execution(
    rt: &HudRuntime,
    expected_session_id: u64,
    is_cancelled: bool,
) -> bool {
    rt.visible
        && rt.phase == HudPhase::Executing
        && rt.session_id == expected_session_id
        && rt.active_run_session_id == Some(expected_session_id)
        && !is_cancelled
}

fn should_fire_no_match_timeout(rt: &HudRuntime, expected_session_id: u64) -> bool {
    rt.visible && rt.phase == HudPhase::Listening && rt.session_id == expected_session_id
}

fn should_fire_auto_dismiss(rt: &HudRuntime, expected_session_id: u64) -> bool {
    rt.visible && rt.phase == HudPhase::Done && rt.session_id == expected_session_id
}

fn touch_speech_activity(rt: &SharedHud) {
    if let Ok(mut s) = rt.lock() {
        if s.visible && s.phase == HudPhase::Listening {
            s.last_speech_activity = Some(Instant::now());
        }
    }
}

fn touch_speech_on_transcript(rt: &SharedHud, text: &str) {
    if !text.trim().is_empty() {
        touch_speech_activity(rt);
    }
}

fn touch_speech_on_amplitude(rt: &SharedHud, amplitude: f64) {
    if amplitude >= SPEECH_AMPLITUDE_THRESHOLD {
        touch_speech_activity(rt);
    }
}

/// Dismiss after `NO_MATCH_TIMEOUT` since **last speech** (transcript or mic), not since HUD opened.
fn spawn_no_match_watchdog(
    app: AppHandle,
    rt: SharedHud,
    audio: SharedAudioPipeline,
    expected_session_id: u64,
) {
    const TICK: Duration = Duration::from_millis(200);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(TICK);
            let should_dismiss = {
                let s = match rt.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                if !should_fire_no_match_timeout(&s, expected_session_id) {
                    return;
                }
                let Some(last) = s.last_speech_activity else {
                    continue;
                };
                last.elapsed() >= NO_MATCH_TIMEOUT
            };
            if should_dismiss {
                break;
            }
        }
        let should = rt
            .lock()
            .map(|s| should_fire_no_match_timeout(&s, expected_session_id))
            .unwrap_or(false);
        if !should {
            return;
        }
        let _ = dismiss_hud(&app, &rt);
        audio::stop_shared_pipeline(&audio);
    });
}

/// After `Done`, wait until **silence** after last speech activity, then run the 4s countdown before dismissing.
fn schedule_auto_dismiss(
    app: AppHandle,
    rt: SharedHud,
    audio: SharedAudioPipeline,
    expected_session_id: u64,
) {
    const TICK: Duration = Duration::from_millis(50);
    std::thread::spawn(move || {
        if let Some(last) = rt.lock().ok().and_then(|s| s.last_speech_activity) {
            let silence_end = last + SILENCE_BEFORE_AUTO_DISMISS;
            while Instant::now() < silence_end {
                std::thread::sleep(TICK);
                if !rt
                    .lock()
                    .map(|s| should_fire_auto_dismiss(&s, expected_session_id))
                    .unwrap_or(false)
                {
                    return;
                }
            }
        }
        std::thread::sleep(AUTO_DISMISS_AFTER);
        let should_dismiss = rt
            .lock()
            .map(|s| should_fire_auto_dismiss(&s, expected_session_id))
            .unwrap_or(false);
        if !should_dismiss {
            return;
        }
        let _ = dismiss_hud(&app, &rt);
        audio::stop_shared_pipeline(&audio);
    });
}

fn spawn_deferred_partial_match(
    app: AppHandle,
    rt: SharedHud,
    audio: SharedAudioPipeline,
    expected_session_id: u64,
    expected_revision: u64,
) {
    std::thread::spawn(move || {
        std::thread::sleep(SILENCE_BEFORE_MATCH);
        let text = {
            let s = match rt.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if !should_attempt_command_match(&s) || s.session_id != expected_session_id {
                return;
            }
            if s.transcript_revision != expected_revision {
                return;
            }
            let Some(last) = s.last_speech_activity else {
                return;
            };
            if last.elapsed() < SILENCE_BEFORE_MATCH {
                return;
            }
            s.pending_transcript.clone()
        };
        let _ = try_match_and_execute(&app, &rt, &audio, &text);
    });
}

fn await_follow_up_input(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
    expected_session_id: u64,
    cancel_flag: &Arc<AtomicBool>,
    _prompt: &str,
) -> Result<String, String> {
    {
        let mut s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
        if !s.visible || s.session_id != expected_session_id || s.phase != HudPhase::Executing {
            return Err(ACTION_RUN_CANCELLED_MSG.to_string());
        }
        s.phase = HudPhase::AwaitingInput;
        s.pending_follow_up_response = None;
        s.pending_follow_up_candidate = None;
        s.pending_follow_up_candidate_at = None;
    }
    sync_hud_window(app, HudPhase::AwaitingInput)?;
    emit_hud_phase(app, HudPhase::AwaitingInput);
    let _ = app.emit("action-status", serde_json::json!({ "text": "follow up" }));
    try_start_listening_audio(app, audio, expected_session_id);

    let deadline = Instant::now() + FOLLOW_UP_TIMEOUT;
    const POLL: Duration = Duration::from_millis(50);
    loop {
        std::thread::sleep(POLL);
        let state = {
            let mut s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
            maybe_promote_follow_up_candidate(&mut s, Instant::now());
            if let Some(response) = take_follow_up_response(&mut s, expected_session_id) {
                return Ok(response);
            }
            should_abort_follow_up_wait(
                &s,
                expected_session_id,
                cancel_flag.load(Ordering::Relaxed),
                Instant::now(),
                deadline,
            )
        };
        match state {
            None => {}
            Some(FollowUpAbortReason::Cancelled) => {
                audio::stop_shared_pipeline(audio);
                return Err(ACTION_RUN_CANCELLED_MSG.to_string());
            }
            Some(FollowUpAbortReason::TimedOut) => {
                audio::stop_shared_pipeline(audio);
                {
                    let mut s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
                    if s.session_id == expected_session_id {
                        s.active_run_cancel = None;
                        s.active_run_session_id = None;
                        s.pending_follow_up_response = None;
                        s.pending_follow_up_candidate = None;
                        s.pending_follow_up_candidate_at = None;
                    }
                }
                let _ = set_phase(app, rt, HudPhase::Done)?;
                return Err(FOLLOW_UP_TIMEOUT_MSG.to_string());
            }
        }
    }
}

fn set_phase(app: &AppHandle, rt: &SharedHud, phase: HudPhase) -> Result<u64, String> {
    debug!("flow: set_phase -> {}", phase.as_str());
    let session_id = {
        let mut s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
        s.phase = phase;
        s.session_id
    };
    sync_hud_window(app, phase)?;
    emit_hud_phase(app, phase);
    Ok(session_id)
}

fn try_match_and_execute(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
    text: &str,
) -> Result<(), String> {
    let command_cache = app.state::<CommandCache>();
    let nodes = read_command_cache(&command_cache)?;
    let default_threshold_pct = load_default_fuzzy_threshold_pct(app);
    let matcher_nodes: Vec<db::CommandNode> = nodes
        .iter()
        .cloned()
        .map(|mut node| {
            node.fuzzy_threshold_pct =
                resolve_fuzzy_threshold_pct(node.fuzzy_threshold_pct, default_threshold_pct);
            node
        })
        .collect();
    let matched = match commands::match_command(text, &matcher_nodes) {
        Some(m) => m,
        None => {
            debug!("flow: no trigger phrase matched");
            return Ok(());
        }
    };

    {
        let s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
        if s.phase != HudPhase::Listening || !s.visible {
            debug!("flow: skip match (phase changed before commit)");
            return Ok(());
        }
    }
    info!(
        "flow: MATCH node_id={} phrase={:?} span={}..{}",
        matched.node_id, matched.matched_phrase, matched.span_start, matched.span_end
    );
    let _ = app.emit("match-result", &matched);
    let _ = set_phase(app, rt, HudPhase::Matched)?;

    debug!("flow: stopping mic pipeline");
    audio::stop_shared_pipeline(audio);
    debug!("flow: mic stopped; phase executing");
    let executing_session_id = set_phase(app, rt, HudPhase::Executing)?;
    if let Some(node) = nodes.iter().find(|n| n.id.to_string() == matched.node_id) {
        let node = node.clone();
        let app_h = app.clone();
        let rt_h = Arc::clone(rt);
        let audio_h = audio.clone();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        {
            let mut s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
            if s.session_id != executing_session_id || s.phase != HudPhase::Executing {
                debug!("flow: skip execute spawn (phase/session changed)");
                return Ok(());
            }
            cancel_active_run_in_state(&mut s);
            s.active_run_cancel = Some(cancel_flag.clone());
            s.active_run_session_id = Some(executing_session_id);
            s.pending_follow_up_response = None;
            s.pending_follow_up_candidate = None;
            s.pending_follow_up_candidate_at = None;
        }
        info!("flow: spawn execute_command for node_id={}", node.id);
        let app_index_snapshot = {
            let st = app.state::<AppIndexStore>();
            let guard = st
                .read()
                .map_err(|_| "app index lock poisoned".to_string())?;
            guard.clone()
        };
        std::thread::spawn(move || {
            let followup_cancel = cancel_flag.clone();
            let app_for_followup = app_h.clone();
            let rt_for_followup = Arc::clone(&rt_h);
            let audio_for_followup = audio_h.clone();
            let runtime = TauriActionRuntime::with_follow_up_handler(
                &app_h,
                cancel_flag.clone(),
                Box::new(move |prompt| {
                    let response = await_follow_up_input(
                        &app_for_followup,
                        &rt_for_followup,
                        &audio_for_followup,
                        executing_session_id,
                        &followup_cancel,
                        prompt,
                    )?;
                    audio::stop_shared_pipeline(&audio_for_followup);
                    let _ = set_phase(&app_for_followup, &rt_for_followup, HudPhase::Executing)?;
                    Ok(response)
                }),
            );
            commands::execute_command(&node, &runtime, Some(app_index_snapshot.as_slice()));
            let should_finalize = {
                let mut s = match rt_h.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                let allowed = should_finalize_execution(
                    &s,
                    executing_session_id,
                    cancel_flag.load(Ordering::Relaxed),
                );
                if allowed {
                    s.active_run_cancel = None;
                    s.active_run_session_id = None;
                }
                allowed
            };
            if !should_finalize {
                return;
            }
            if let Ok(done_session_id) = set_phase(&app_h, &rt_h, HudPhase::Done) {
                debug!("flow: scheduled auto-dismiss session_id={done_session_id}");
                schedule_auto_dismiss(app_h.clone(), Arc::clone(&rt_h), audio_h, done_session_id);
            }
        });
    } else {
        warn!(
            "flow: matched node_id={} but no row in loaded nodes (count={})",
            matched.node_id,
            nodes.len()
        );
    }
    Ok(())
}

/// # Transcription → recognition → action
///
/// 1. **STT** (`audio/stt.rs`): while the mic runs, emits `transcript-update` with partial text
///    (`is_final: false`). After capture stops, may emit one final (`is_final: true`).
/// 2. **Orchestrator** (this function): if HUD is `listening` and text is non-empty, run substring
///    match against SQLite command nodes (`commands::matcher`).
/// 3. On match: emit `match-result` to the HUD → phases **matched** → **executing** →
///    [`audio::stop_shared_pipeline`] (releases mutex before drop) → spawn [`commands::execute_command`]
///    (`OpenApp` / `OpenUrl`) → phase **done** → [`schedule_auto_dismiss`].
/// 4. **React** (`subscribeHudIpc`): applies events to Zustand; transcript + span highlight from
///    `match-result`; status line from `action-status`.
fn process_transcript_update(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
    payload: &str,
) -> Result<(), String> {
    let update: audio::stt::TranscriptUpdate = serde_json::from_str(payload)
        .map_err(|e| format!("invalid transcript-update payload: {e}"))?;
    debug!(
        "flow: transcript-update is_final={} session={} len={} preview={:?}",
        update.is_final,
        update.hud_session_id,
        update.text.chars().count(),
        preview_chars(&update.text, 72)
    );

    let (can_match, can_attempt_this_update) = {
        let s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
        if update.hud_session_id != s.session_id {
            debug!(
                "flow: skip (stale transcript session id={} current={})",
                update.hud_session_id, s.session_id
            );
            return Ok(());
        }
        (
            should_attempt_command_match(&s),
            should_attempt_match_for_update(&s, update.is_final),
        )
    };
    if capture_follow_up_from_update(rt, &update) {
        return Ok(());
    }
    touch_speech_on_transcript(rt, &update.text);
    let pending_meta = update_pending_transcript(rt, &update.text);
    if !can_match {
        debug!("flow: skip (not listening or not visible)");
        return Ok(());
    }
    if update.text.trim().is_empty() {
        debug!("flow: skip (empty transcript)");
        return Ok(());
    }
    if !can_attempt_this_update {
        if let Some((session_id, revision)) = pending_meta {
            debug!(
                "flow: defer match until silence window session={} rev={}",
                session_id, revision
            );
            spawn_deferred_partial_match(
                app.clone(),
                Arc::clone(rt),
                audio.clone(),
                session_id,
                revision,
            );
        } else {
            debug!("flow: defer match until silence window");
        }
        return Ok(());
    }
    try_match_and_execute(app, rt, audio, &update.text)
}

fn show_hud_from_hotkey(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
    is_paused: &Arc<AtomicBool>,
) -> Result<(), String> {
    let mut s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))?;

    let mut listening_session_id: Option<u64> = None;
    if !s.visible {
        let sid = prepare_hud_listening_session(&mut s);
        listening_session_id = Some(sid);
        window.center().map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    } else {
        prepare_hud_close_session(&mut s);
        window.hide().map_err(|e| e.to_string())?;
    }

    let phase = s.phase;
    let session_id = s.session_id;
    drop(s);

    sync_hud_window(app, phase)?;
    emit_hud_phase(app, phase);

    if tray::mic_start_allowed(is_paused, phase) {
        try_start_listening_audio(app, audio, session_id);
        if let Some(sid) = listening_session_id.or(Some(session_id)) {
            spawn_no_match_watchdog(app.clone(), Arc::clone(rt), audio.clone(), sid);
        }
    } else {
        audio::stop_shared_pipeline(audio);
    }

    Ok(())
}

/// Wake word: open HUD + listening when hidden; never toggles closed (unlike hotkey).
fn wake_request_hud(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
    is_paused: &Arc<AtomicBool>,
) -> Result<(), String> {
    let mut s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
    if s.visible {
        return Ok(());
    }
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))?;

    let session_id = prepare_hud_listening_session(&mut s);
    window.center().map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    drop(s);

    sync_hud_window(app, HudPhase::Listening)?;
    emit_hud_phase(app, HudPhase::Listening);

    if tray::mic_start_allowed(is_paused, HudPhase::Listening) {
        try_start_listening_audio(app, audio, session_id);
        spawn_no_match_watchdog(app.clone(), Arc::clone(rt), audio.clone(), session_id);
    }
    Ok(())
}

fn make_wake_callback(
    wake_app: AppHandle,
    wake_hud: SharedHud,
    wake_audio: SharedAudioPipeline,
    wake_paused: Arc<AtomicBool>,
) -> Arc<dyn Fn() + Send + Sync + 'static> {
    Arc::new(move || {
        let app = wake_app.clone();
        let hud = Arc::clone(&wake_hud);
        let aud = wake_audio.clone();
        let p = Arc::clone(&wake_paused);
        let app_for_inner = app.clone();
        if let Err(err) = app.run_on_main_thread(move || {
            if let Err(e) = wake_request_hud(&app_for_inner, &hud, &aud, &p) {
                warn!("wake_request_hud: {e}");
            }
        }) {
            warn!("wake main-thread dispatch: {err:?}");
        }
    })
}

/// Porcupine / OWW use a dedicated wake thread; `hotkey` does not.
pub(crate) fn wake_engine_uses_dedicated_thread(wake_engine: &str) -> bool {
    matches!(wake_engine, "porcupine" | "oww")
}

fn try_start_wake_supervisor(
    app: &AppHandle,
    resource_dir: std::path::PathBuf,
    settings: &db::AppSettings,
    hud: &SharedHud,
    audio: &SharedAudioPipeline,
    is_paused: &Arc<AtomicBool>,
) -> Result<Option<audio::wake::thread::WakeSupervisor>, String> {
    if !wake_engine_uses_dedicated_thread(settings.wake_engine.as_str()) {
        return Ok(None);
    }
    let cb = make_wake_callback(
        app.clone(),
        Arc::clone(hud),
        audio.clone(),
        Arc::clone(is_paused),
    );
    audio::wake::thread::spawn_wake_thread(
        app.clone(),
        resource_dir,
        settings.wake_engine.as_str(),
        settings,
        is_paused.clone(),
        cb,
    )
    .map(Some)
}

fn wake_reload_from_settings(
    app: &AppHandle,
    slot: &WakeSupervisorState,
    resource_dir: std::path::PathBuf,
    settings: &db::AppSettings,
    hud: &SharedHud,
    audio: &SharedAudioPipeline,
    is_paused: &Arc<AtomicBool>,
) {
    if let Ok(mut g) = slot.0.lock() {
        if let Some(prev) = g.take() {
            prev.shutdown();
        }
    }
    match try_start_wake_supervisor(app, resource_dir, settings, hud, audio, is_paused) {
        Ok(Some(s)) => {
            if let Ok(mut g) = slot.0.lock() {
                *g = Some(s);
            }
        }
        Ok(None) => {}
        Err(e) => warn!("wake thread not started: {e}"),
    }
}

fn settings_patch_triggers_wake_reload(
    patch: &db::SettingsPatch,
    settings_after: &db::AppSettings,
) -> bool {
    if patch.wake_engine.is_some() {
        return true;
    }
    patch.oww_threshold.is_some() && settings_after.wake_engine == "oww"
}

fn dismiss_hud(app: &AppHandle, rt: &SharedHud) -> Result<(), String> {
    let mut s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))?;

    s.phase = HudPhase::Stopped;
    s.visible = false;
    s.session_id = s.session_id.wrapping_add(1);
    s.pending_transcript.clear();
    s.transcript_revision = 0;
    s.pending_follow_up_response = None;
    s.pending_follow_up_candidate = None;
    s.pending_follow_up_candidate_at = None;
    cancel_active_run_in_state(&mut s);
    window.hide().map_err(|e| e.to_string())?;

    let phase = s.phase;
    drop(s);

    sync_hud_window(app, phase)?;
    emit_hud_phase(app, phase);
    Ok(())
}

#[tauri::command]
fn hud_get_phase(state: State<'_, SharedHud>) -> Result<HudPhase, String> {
    let s = state.lock().map_err(|_| "hud state poisoned".to_string())?;
    Ok(s.phase)
}

#[tauri::command]
fn hud_set_phase(
    phase: HudPhase,
    app: AppHandle,
    state: State<'_, SharedHud>,
) -> Result<(), String> {
    {
        let mut s = state.lock().map_err(|_| "hud state poisoned".to_string())?;
        s.phase = phase;
        match phase {
            HudPhase::Listening => {
                s.visible = true;
                s.session_id = s.session_id.wrapping_add(1);
                s.last_speech_activity = Some(Instant::now());
                s.pending_transcript.clear();
                s.transcript_revision = 0;
                s.pending_follow_up_response = None;
                s.pending_follow_up_candidate = None;
                s.pending_follow_up_candidate_at = None;
                cancel_active_run_in_state(&mut s);
            }
            HudPhase::Stopped => {
                s.visible = false;
                s.session_id = s.session_id.wrapping_add(1);
                s.pending_transcript.clear();
                s.transcript_revision = 0;
                s.pending_follow_up_response = None;
                s.pending_follow_up_candidate = None;
                s.pending_follow_up_candidate_at = None;
                cancel_active_run_in_state(&mut s);
            }
            _ => {}
        }
    }
    sync_hud_window(&app, phase)?;
    emit_hud_phase(&app, phase);
    Ok(())
}

#[tauri::command]
fn hud_dismiss(
    app: AppHandle,
    state: State<'_, SharedHud>,
    audio: State<'_, SharedAudioPipeline>,
) -> Result<(), String> {
    dismiss_hud(&app, &state)?;
    audio::stop_shared_pipeline(&audio);
    Ok(())
}

#[tauri::command]
fn open_editor(app: AppHandle) -> Result<(), String> {
    open_or_create_editor_window(&app)
}

#[tauri::command]
fn list_commands(app: AppHandle) -> Result<Vec<db::CommandNode>, String> {
    let conn = open_db_connection(&app)?;
    db::get_all_commands(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_command(app: AppHandle, id: i64) -> Result<Option<db::CommandNode>, String> {
    let conn = open_db_connection(&app)?;
    db::get_command_by_id(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
fn create_command(
    app: AppHandle,
    node: CommandNodePayload,
    command_cache: State<'_, CommandCache>,
) -> Result<db::CommandNode, String> {
    let conn = open_db_connection(&app)?;
    let row = node.try_into_new_command_node()?;
    let id = db::insert_command(&conn, &row).map_err(|e| e.to_string())?;
    let saved = db::get_command_by_id(&conn, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("created node {id} was not found"))?;
    refresh_command_cache(&app, &command_cache)?;
    emit_editor_commands_changed(&app);
    Ok(saved)
}

#[tauri::command]
fn update_command(
    app: AppHandle,
    id: i64,
    node: CommandNodePayload,
    command_cache: State<'_, CommandCache>,
) -> Result<db::CommandNode, String> {
    let conn = open_db_connection(&app)?;
    let row = node.try_into_new_command_node()?;
    let changed = db::update_command(&conn, id, &row).map_err(|e| e.to_string())?;
    if !changed {
        return Err(format!("command with id {id} was not found"));
    }
    let saved = db::get_command_by_id(&conn, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("updated node {id} was not found"))?;
    refresh_command_cache(&app, &command_cache)?;
    emit_editor_commands_changed(&app);
    Ok(saved)
}

#[tauri::command]
fn delete_command(
    app: AppHandle,
    id: i64,
    command_cache: State<'_, CommandCache>,
) -> Result<bool, String> {
    let conn = open_db_connection(&app)?;
    let deleted = db::delete_command(&conn, id).map_err(|e| e.to_string())?;
    if deleted {
        refresh_command_cache(&app, &command_cache)?;
        emit_editor_commands_changed(&app);
    }
    Ok(deleted)
}

#[tauri::command]
fn reorder_commands(
    app: AppHandle,
    payload: ReorderCommandsPayload,
    command_cache: State<'_, CommandCache>,
) -> Result<(), String> {
    let conn = open_db_connection(&app)?;
    db::reorder_commands(&conn, &payload.ordered_ids).map_err(|e| e.to_string())?;
    refresh_command_cache(&app, &command_cache)?;
    emit_editor_commands_changed(&app);
    Ok(())
}

#[tauri::command]
fn get_setting(app: AppHandle, key: String) -> Result<Option<String>, String> {
    let normalized_key = validate_setting_key(&key)?;
    let conn = open_db_connection(&app)?;
    db::get_setting(&conn, &normalized_key).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_setting(app: AppHandle, key: String, value: String) -> Result<(), String> {
    let normalized_key = validate_setting_key(&key)?;
    let conn = open_db_connection(&app)?;
    db::set_setting(&conn, &normalized_key, value.trim()).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_hotkey(
    app: AppHandle,
    hotkey: String,
    hotkey_state: State<'_, HotkeyBindingState>,
) -> Result<String, String> {
    let next_hotkey = normalize_hotkey_input(&hotkey)?;
    let conn = open_db_connection(&app)?;
    let mut current_hotkey = hotkey_state
        .current
        .lock()
        .map_err(|_| "hotkey state poisoned".to_string())?;
    let existing_hotkey = current_hotkey.clone();
    if existing_hotkey == next_hotkey {
        db::set_setting(&conn, SETTING_KEY_HOTKEY, &next_hotkey).map_err(|e| e.to_string())?;
        return Ok(next_hotkey);
    }

    app.global_shortcut()
        .unregister(existing_hotkey.as_str())
        .map_err(|e| format!("failed to unregister current hotkey `{existing_hotkey}`: {e}"))?;

    if let Err(register_error) = app.global_shortcut().register(next_hotkey.as_str()) {
        let _ = app.global_shortcut().register(existing_hotkey.as_str());
        return Err(format!(
            "failed to register new hotkey `{next_hotkey}`: {register_error}"
        ));
    }

    if let Err(persist_error) = db::set_setting(&conn, SETTING_KEY_HOTKEY, &next_hotkey) {
        let _ = app.global_shortcut().unregister(next_hotkey.as_str());
        let _ = app.global_shortcut().register(existing_hotkey.as_str());
        return Err(format!("failed to persist hotkey: {persist_error}"));
    }

    *current_hotkey = next_hotkey.clone();
    Ok(next_hotkey)
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Result<db::AppSettings, String> {
    let conn = open_db_connection(&app)?;
    db::get_app_settings(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_settings(
    app: AppHandle,
    patch: db::SettingsPatch,
    wake_slot: State<'_, WakeSupervisorState>,
    hud_state: State<'_, SharedHud>,
    audio: State<'_, SharedAudioPipeline>,
    is_paused: State<'_, Arc<AtomicBool>>,
) -> Result<db::AppSettings, String> {
    let conn = open_db_connection(&app)?;
    db::apply_settings_patch(&conn, &patch).map_err(|e| e.to_string())?;
    let settings = db::get_app_settings(&conn).map_err(|e| e.to_string())?;
    if settings_patch_triggers_wake_reload(&patch, &settings) {
        let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
        wake_reload_from_settings(
            &app,
            wake_slot.inner(),
            resource_dir,
            &settings,
            hud_state.inner(),
            audio.inner(),
            is_paused.inner(),
        );
    }
    Ok(settings)
}

#[tauri::command]
fn save_api_key(
    app: AppHandle,
    service: String,
    key: String,
    wake_slot: State<'_, WakeSupervisorState>,
    hud_state: State<'_, SharedHud>,
    audio: State<'_, SharedAudioPipeline>,
    is_paused: State<'_, Arc<AtomicBool>>,
) -> Result<(), String> {
    let normalized = service.trim().to_ascii_lowercase();
    keychain::save_api_key(&normalized, &key)?;
    let conn = open_db_connection(&app)?;
    db::set_key_stored_flag(&conn, &normalized, true).map_err(|e| e.to_string())?;
    if normalized == "porcupine" {
        let conn = open_db_connection(&app)?;
        let settings = db::get_app_settings(&conn).map_err(|e| e.to_string())?;
        if settings.wake_engine == "porcupine" {
            let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
            wake_reload_from_settings(
                &app,
                wake_slot.inner(),
                resource_dir,
                &settings,
                hud_state.inner(),
                audio.inner(),
                is_paused.inner(),
            );
        }
    }
    Ok(())
}

#[tauri::command]
fn delete_api_key(
    app: AppHandle,
    service: String,
    wake_slot: State<'_, WakeSupervisorState>,
    hud_state: State<'_, SharedHud>,
    audio: State<'_, SharedAudioPipeline>,
    is_paused: State<'_, Arc<AtomicBool>>,
) -> Result<(), String> {
    let normalized = service.trim().to_ascii_lowercase();
    keychain::delete_api_key(&normalized)?;
    let conn = open_db_connection(&app)?;
    db::set_key_stored_flag(&conn, &normalized, false).map_err(|e| e.to_string())?;
    if normalized == "porcupine" {
        let conn = open_db_connection(&app)?;
        let settings = db::get_app_settings(&conn).map_err(|e| e.to_string())?;
        if settings.wake_engine == "porcupine" {
            let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
            wake_reload_from_settings(
                &app,
                wake_slot.inner(),
                resource_dir,
                &settings,
                hud_state.inner(),
                audio.inner(),
                is_paused.inner(),
            );
        }
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    let hud_state: SharedHud = Arc::new(Mutex::new(HudRuntime::default()));
    // cpal stream is !Send; SharedAudioPipeline uses unsafe Send/Sync — see audio/mod.rs
    #[allow(clippy::arc_with_non_send_sync)]
    let audio_pipeline = SharedAudioPipeline(Arc::new(Mutex::new(None)));
    let is_paused = Arc::new(AtomicBool::new(false));
    let command_cache: CommandCache = Arc::new(RwLock::new(Vec::new()));
    let app_index_store: AppIndexStore = Arc::new(RwLock::new(Vec::new()));

    tauri::Builder::default()
        .manage(Arc::clone(&hud_state))
        .manage(audio_pipeline.clone())
        .manage(Arc::clone(&is_paused))
        .manage(command_cache.clone())
        .manage(app_index_store.clone())
        .manage(HotkeyBindingState {
            current: Mutex::new(DEFAULT_HOTKEY.to_string()),
        })
        .manage(WakeSupervisorState::default())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .setup({
            let hud_state = Arc::clone(&hud_state);
            let audio_for_shortcut = audio_pipeline.clone();
            let is_paused_for_shortcut = Arc::clone(&is_paused);
            let command_cache_for_setup = command_cache.clone();
            let app_index_for_setup = app_index_store.clone();
            move |app| {
                let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                db::init_db(&dir.join("jarvis.db")).map_err(|e| e.to_string())?;
                refresh_app_index_on_startup(app.handle(), &app_index_for_setup)?;
                refresh_command_cache(app.handle(), &command_cache_for_setup)?;
                let conn = open_db_connection(app.handle())?;
                let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
                let app_settings = db::get_app_settings(&conn).map_err(|e| e.to_string())?;
                let wake_slot = app.state::<WakeSupervisorState>();
                let app_h = app.handle().clone();
                match try_start_wake_supervisor(
                    &app_h,
                    resource_dir,
                    &app_settings,
                    &hud_state,
                    &audio_for_shortcut,
                    &is_paused_for_shortcut,
                ) {
                    Ok(Some(s)) => {
                        if let Ok(mut g) = wake_slot.0.lock() {
                            *g = Some(s);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => warn!("wake thread not started: {e}"),
                }
                let configured_hotkey =
                    match db::get_setting(&conn, SETTING_KEY_HOTKEY).map_err(|e| e.to_string())? {
                        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
                        _ => {
                            db::set_setting(&conn, SETTING_KEY_HOTKEY, DEFAULT_HOTKEY)
                                .map_err(|e| e.to_string())?;
                            DEFAULT_HOTKEY.to_string()
                        }
                    };
                {
                    let hotkey_state = app.state::<HotkeyBindingState>();
                    let mut current = hotkey_state
                        .current
                        .lock()
                        .map_err(|_| "hotkey state poisoned".to_string())?;
                    *current = configured_hotkey.clone();
                }

                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                {
                    tray::setup_tray(
                        app.handle(),
                        Arc::clone(&is_paused_for_shortcut),
                        audio_for_shortcut.clone(),
                    )
                    .map_err(|e| e.to_string())?;
                }

                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                {
                    let transcript_hud = Arc::clone(&hud_state);
                    let transcript_audio = audio_for_shortcut.clone();
                    let transcript_app = app.handle().clone();
                    app.listen("transcript-update", move |event| {
                        if let Err(err) = process_transcript_update(
                            &transcript_app,
                            &transcript_hud,
                            &transcript_audio,
                            event.payload(),
                        ) {
                            let _ = transcript_app
                                .emit("audio-error", serde_json::json!({ "message": err }));
                        }
                    });

                    let amp_hud = Arc::clone(&hud_state);
                    app.listen("amplitude-update", move |event| {
                        let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload())
                        else {
                            return;
                        };
                        let Some(a) = v.get("amplitude").and_then(|x| x.as_f64()) else {
                            return;
                        };
                        touch_speech_on_amplitude(&amp_hud, a);
                    });

                    app.handle()
                        .plugin(
                            ShortcutBuilder::new()
                                .with_shortcuts([configured_hotkey.as_str()])
                                .map_err(|e| e.to_string())?
                                .with_handler({
                                    let hud_state = Arc::clone(&hud_state);
                                    let audio_for_shortcut = audio_for_shortcut.clone();
                                    let is_paused_for_shortcut =
                                        Arc::clone(&is_paused_for_shortcut);
                                    move |app, _shortcut, event| {
                                        if event.state != ShortcutState::Pressed {
                                            return;
                                        }
                                        let _ = show_hud_from_hotkey(
                                            app,
                                            &hud_state,
                                            &audio_for_shortcut,
                                            &is_paused_for_shortcut,
                                        );
                                    }
                                })
                                .build(),
                        )
                        .map_err(|e| e.to_string())?;
                }

                sync_hud_window(app.handle(), HudPhase::Idle).map_err(|e| e.to_string())?;
                emit_hud_phase(app.handle(), HudPhase::Idle);
                Ok(())
            }
        })
        .invoke_handler(tauri::generate_handler![
            hud_get_phase,
            hud_set_phase,
            hud_dismiss,
            open_editor,
            list_commands,
            get_command,
            create_command,
            update_command,
            delete_command,
            reorder_commands,
            get_setting,
            set_setting,
            set_hotkey,
            get_settings,
            update_settings,
            save_api_key,
            delete_api_key
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::Action;
    use rusqlite::Connection;
    use std::sync::atomic::Ordering;
    use tempfile::tempdir;

    #[test]
    fn open_editor_focuses_existing_window_instead_of_duplicate() {
        assert!(should_focus_existing_editor_window(true));
        assert!(!should_focus_existing_editor_window(false));
    }

    fn sample_settings(wake_engine: &str) -> db::AppSettings {
        db::AppSettings {
            porcupine_key_stored: false,
            wake_engine: wake_engine.into(),
            oww_threshold: 0.5,
            stt_provider: "local".into(),
            remote_stt_url: String::new(),
            remote_stt_model: None,
            remote_stt_timeout_secs: 30,
            remote_stt_key_stored: false,
        }
    }

    #[test]
    fn wake_engine_uses_dedicated_thread_only_for_porcupine_and_oww() {
        assert!(!wake_engine_uses_dedicated_thread("hotkey"));
        assert!(wake_engine_uses_dedicated_thread("porcupine"));
        assert!(wake_engine_uses_dedicated_thread("oww"));
    }

    #[test]
    fn settings_patch_wake_engine_triggers_wake_reload() {
        let patch = db::SettingsPatch {
            wake_engine: Some("porcupine".into()),
            oww_threshold: None,
            stt_provider: None,
            remote_stt_url: None,
            remote_stt_model: None,
            remote_stt_timeout_secs: None,
        };
        assert!(settings_patch_triggers_wake_reload(
            &patch,
            &sample_settings("porcupine")
        ));
    }

    #[test]
    fn oww_threshold_patch_triggers_reload_only_when_engine_is_oww() {
        let patch = db::SettingsPatch {
            wake_engine: None,
            oww_threshold: Some(0.6),
            stt_provider: None,
            remote_stt_url: None,
            remote_stt_model: None,
            remote_stt_timeout_secs: None,
        };
        assert!(!settings_patch_triggers_wake_reload(
            &patch,
            &sample_settings("hotkey")
        ));
        assert!(settings_patch_triggers_wake_reload(
            &patch,
            &sample_settings("oww")
        ));
    }

    #[test]
    fn command_match_requires_listening_visible() {
        let mut rt = HudRuntime::default();
        rt.visible = true;
        rt.phase = HudPhase::Listening;
        assert!(should_attempt_command_match(&rt));
        rt.phase = HudPhase::Done;
        assert!(!should_attempt_command_match(&rt));
        rt.phase = HudPhase::Listening;
        rt.visible = false;
        assert!(!should_attempt_command_match(&rt));
    }

    #[test]
    fn partial_transcript_match_is_debounced_while_speaking() {
        let mut rt = HudRuntime::default();
        rt.last_speech_activity = Some(Instant::now());
        assert!(!should_attempt_match_for_update(&rt, false));
    }

    #[test]
    fn partial_transcript_match_allowed_after_silence_gap() {
        let mut rt = HudRuntime::default();
        rt.last_speech_activity =
            Some(Instant::now() - SILENCE_BEFORE_MATCH - Duration::from_millis(1));
        assert!(should_attempt_match_for_update(&rt, false));
    }

    #[test]
    fn final_transcript_bypasses_match_debounce() {
        let mut rt = HudRuntime::default();
        rt.last_speech_activity = Some(Instant::now());
        assert!(should_attempt_match_for_update(&rt, true));
    }

    #[test]
    fn no_match_timeout_requires_same_session_and_listening() {
        let mut rt = HudRuntime::default();
        rt.visible = true;
        rt.phase = HudPhase::Listening;
        rt.session_id = 7;
        assert!(should_fire_no_match_timeout(&rt, 7));
        assert!(!should_fire_no_match_timeout(&rt, 8));
        rt.phase = HudPhase::Done;
        assert!(!should_fire_no_match_timeout(&rt, 7));
    }

    /// Watchdog fires only once `last.elapsed()` exceeds the window (speech resets `last`).
    #[test]
    fn no_match_fires_when_idle_since_last_speech_exceeds_timeout() {
        let last = Instant::now() - NO_MATCH_TIMEOUT - Duration::from_millis(50);
        assert!(last.elapsed() >= NO_MATCH_TIMEOUT);
    }

    #[test]
    fn follow_up_happy_path_consumes_response_once() {
        let mut rt = HudRuntime::default();
        rt.visible = true;
        rt.phase = HudPhase::AwaitingInput;
        rt.session_id = 42;
        rt.pending_follow_up_response = Some("open docs".to_string());

        let response = take_follow_up_response(&mut rt, 42);
        assert_eq!(response, Some("open docs".to_string()));
        assert_eq!(rt.pending_follow_up_response, None);
    }

    #[test]
    fn follow_up_candidate_promotes_after_stable_silence() {
        let mut rt = HudRuntime::default();
        rt.pending_follow_up_candidate = Some("rust tauri".to_string());
        rt.pending_follow_up_candidate_at = Some(Instant::now() - SILENCE_BEFORE_MATCH);
        maybe_promote_follow_up_candidate(&mut rt, Instant::now());
        assert_eq!(
            rt.pending_follow_up_response,
            Some("rust tauri".to_string())
        );
        assert_eq!(rt.pending_follow_up_candidate, None);
    }

    #[test]
    fn follow_up_wait_times_out_after_deadline() {
        let mut rt = HudRuntime::default();
        rt.visible = true;
        rt.phase = HudPhase::AwaitingInput;
        rt.session_id = 7;

        let now = Instant::now();
        let deadline = now - Duration::from_millis(1);
        assert_eq!(
            should_abort_follow_up_wait(&rt, 7, false, now, deadline),
            Some(FollowUpAbortReason::TimedOut)
        );
    }

    #[test]
    fn follow_up_wait_cancels_on_escape_or_session_change() {
        let mut rt = HudRuntime::default();
        rt.visible = true;
        rt.phase = HudPhase::AwaitingInput;
        rt.session_id = 3;
        let now = Instant::now();
        let deadline = now + Duration::from_secs(5);

        assert_eq!(
            should_abort_follow_up_wait(&rt, 3, true, now, deadline),
            Some(FollowUpAbortReason::Cancelled)
        );
        assert_eq!(
            should_abort_follow_up_wait(&rt, 4, false, now, deadline),
            Some(FollowUpAbortReason::Cancelled)
        );
    }

    #[test]
    fn finalize_execution_requires_matching_active_session() {
        let mut rt = HudRuntime::default();
        rt.visible = true;
        rt.phase = HudPhase::Executing;
        rt.session_id = 11;
        rt.active_run_session_id = Some(11);
        assert!(should_finalize_execution(&rt, 11, false));
        assert!(!should_finalize_execution(&rt, 11, true));
        assert!(!should_finalize_execution(&rt, 12, false));
        rt.phase = HudPhase::Done;
        assert!(!should_finalize_execution(&rt, 11, false));
    }

    #[test]
    fn cancel_active_run_sets_flag_and_clears_tracking() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut rt = HudRuntime::default();
        rt.active_run_cancel = Some(cancel.clone());
        rt.active_run_session_id = Some(2);
        cancel_active_run_in_state(&mut rt);
        assert!(cancel.load(Ordering::Relaxed));
        assert!(rt.active_run_cancel.is_none());
        assert_eq!(rt.active_run_session_id, None);
    }

    #[test]
    fn command_payload_validation_rejects_empty_name() {
        let payload = CommandNodePayload {
            name: "   ".into(),
            trigger_phrases: vec!["open notepad".into()],
            actions: vec![ActionPayload::OpenApp {
                name: "notepad".into(),
                path: "notepad.exe".into(),
            }],
            enabled: true,
            fuzzy_threshold_pct: 80,
        };
        assert!(validate_command_node_payload(&payload).is_err());
    }

    #[test]
    fn command_payload_validation_rejects_empty_trigger_list() {
        let payload = CommandNodePayload {
            name: "Open".into(),
            trigger_phrases: vec![],
            actions: vec![ActionPayload::OpenUrl {
                url: "https://example.com".into(),
            }],
            enabled: true,
            fuzzy_threshold_pct: 80,
        };
        assert!(validate_command_node_payload(&payload).is_err());
    }

    #[test]
    fn command_payload_validation_rejects_out_of_range_threshold() {
        let payload = CommandNodePayload {
            name: "Open".into(),
            trigger_phrases: vec!["open".into()],
            actions: vec![ActionPayload::Speak {
                text: "done".into(),
            }],
            enabled: true,
            fuzzy_threshold_pct: 101,
        };
        assert!(validate_command_node_payload(&payload).is_err());
    }

    #[test]
    fn command_payload_conversion_trims_and_round_trips() {
        let payload = CommandNodePayload {
            name: "  Open App  ".into(),
            trigger_phrases: vec!["  open app ".into(), "launch app".into()],
            actions: vec![
                ActionPayload::Wait { ms: 250 },
                ActionPayload::Speak {
                    text: "done".into(),
                },
            ],
            enabled: false,
            fuzzy_threshold_pct: 90,
        };
        let node = payload.try_into_new_command_node().expect("valid payload");
        assert_eq!(node.name, "Open App");
        assert_eq!(
            node.trigger_phrases,
            vec!["open app".to_string(), "launch app".to_string()]
        );
        assert_eq!(node.enabled, false);
        assert_eq!(node.fuzzy_threshold_pct, 90);
        assert_eq!(
            node.actions,
            vec![
                Action::Wait { ms: 250 },
                Action::Speak {
                    text: "done".into()
                }
            ]
        );
    }

    #[test]
    fn default_threshold_applies_when_node_threshold_is_zero() {
        assert_eq!(resolve_fuzzy_threshold_pct(0, 77), 77);
        assert_eq!(resolve_fuzzy_threshold_pct(88, 77), 88);
    }

    #[test]
    fn refresh_command_cache_replaces_previous_snapshot() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("cache-test.db");
        db::init_db(&path).expect("init db");
        let conn = Connection::open(&path).expect("open db");

        let inserted_id = db::insert_command(
            &conn,
            &db::NewCommandNode {
                name: "Cache Node".into(),
                trigger_phrases: vec!["cache me".into()],
                actions: vec![Action::Wait { ms: 25 }],
                enabled: true,
                fuzzy_threshold_pct: 80,
            },
        )
        .expect("insert");
        let inserted = db::get_command_by_id(&conn, inserted_id)
            .expect("get inserted")
            .expect("inserted row");

        let cache: CommandCache = Arc::new(std::sync::RwLock::new(vec![]));
        refresh_command_cache_from_rows(&cache, vec![inserted.clone()]).expect("prime cache");
        assert_eq!(read_command_cache(&cache).expect("read first").len(), 1);

        refresh_command_cache_from_rows(&cache, Vec::new()).expect("replace with empty");
        assert!(
            read_command_cache(&cache).expect("read second").is_empty(),
            "cache should be replaced, not appended"
        );
    }
}

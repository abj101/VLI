mod audio;
mod commands;
mod db;
mod hud;
mod tray;

use audio::SharedAudioPipeline;
use commands::TauriActionRuntime;
use hud::{sync_hud_window, HudPhase, HUD_WINDOW_LABEL};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Listener, Manager, State};
use tauri_plugin_global_shortcut::{Builder as ShortcutBuilder, ShortcutState};

const AUTO_DISMISS_AFTER: Duration = Duration::from_secs(4);
const NO_MATCH_TIMEOUT: Duration = Duration::from_secs(5);
/// After the last speech-related activity, wait this long before treating speech as finished
/// (starts the 4s auto-dismiss countdown only after this gap).
const SILENCE_BEFORE_AUTO_DISMISS: Duration = Duration::from_millis(450);
/// Amplitude above this (0..1) counts as speech for activity / silence detection.
const SPEECH_AMPLITUDE_THRESHOLD: f64 = 0.02;

#[derive(Debug, Default)]
struct HudRuntime {
    phase: HudPhase,
    visible: bool,
    session_id: u64,
    /// Last time we saw speech (transcript text or mic level). Used so timers run on silence, not wall-clock from HUD open.
    last_speech_activity: Option<Instant>,
}

type SharedHud = Arc<Mutex<HudRuntime>>;

fn try_start_listening_audio(app: &AppHandle, slot: &SharedAudioPipeline) {
    let old = {
        let mut g = slot.lock().unwrap();
        g.take()
    };
    drop(old);

    match audio::AudioPipeline::start(app) {
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
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let db_path = dir.join("jarvis.db");
    let conn = rusqlite::Connection::open(db_path).map_err(|e| e.to_string())?;
    db::get_all_commands(&conn).map_err(|e| e.to_string())
}

/// Live STT emits partial `transcript-update` (`is_final: false`). `is_final` is only set when the
/// mic pipeline stops, so matching must run on partials while listening — otherwise commands never fire.
fn should_attempt_command_match(rt: &HudRuntime) -> bool {
    rt.visible && rt.phase == HudPhase::Listening
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
        if let Some(last) = rt
            .lock()
            .ok()
            .and_then(|s| s.last_speech_activity)
        {
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

fn set_phase(app: &AppHandle, rt: &SharedHud, phase: HudPhase) -> Result<u64, String> {
    let session_id = {
        let mut s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
        s.phase = phase;
        s.session_id
    };
    sync_hud_window(app, phase)?;
    emit_hud_phase(app, phase);
    Ok(session_id)
}

fn process_transcript_update(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
    payload: &str,
) -> Result<(), String> {
    let update: audio::stt::TranscriptUpdate = serde_json::from_str(payload)
        .map_err(|e| format!("invalid transcript-update payload: {e}"))?;
    touch_speech_on_transcript(rt, &update.text);

    let can_match = {
        let s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
        should_attempt_command_match(&s)
    };
    if !can_match {
        return Ok(());
    }
    if update.text.trim().is_empty() {
        return Ok(());
    }

    let nodes = load_all_commands(app)?;
    let matched = match commands::match_command(&update.text, &nodes) {
        Some(m) => m,
        None => return Ok(()),
    };

    {
        let s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
        if s.phase != HudPhase::Listening || !s.visible {
            return Ok(());
        }
    }
    let _ = app.emit("match-result", &matched);
    let _ = set_phase(app, rt, HudPhase::Matched)?;

    audio::stop_shared_pipeline(audio);
    let _ = set_phase(app, rt, HudPhase::Executing)?;
    if let Some(node) = nodes.iter().find(|n| n.id.to_string() == matched.node_id) {
        let node = node.clone();
        let app_h = app.clone();
        std::thread::spawn(move || {
            commands::execute_command(&node, &TauriActionRuntime::new(&app_h));
        });
    }

    let done_session_id = set_phase(app, rt, HudPhase::Done)?;
    schedule_auto_dismiss(app.clone(), Arc::clone(rt), audio.clone(), done_session_id);
    Ok(())
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
        s.visible = true;
        s.phase = HudPhase::Listening;
        s.session_id = s.session_id.wrapping_add(1);
        s.last_speech_activity = Some(Instant::now());
        listening_session_id = Some(s.session_id);
        window.center().map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    } else {
        s.phase = HudPhase::Stopped;
        s.visible = false;
        s.session_id = s.session_id.wrapping_add(1);
        window.hide().map_err(|e| e.to_string())?;
    }

    let phase = s.phase;
    let session_id = s.session_id;
    drop(s);

    sync_hud_window(app, phase)?;
    emit_hud_phase(app, phase);

    if tray::mic_start_allowed(is_paused, phase) {
        try_start_listening_audio(app, audio);
        if let Some(sid) = listening_session_id.or(Some(session_id)) {
            spawn_no_match_watchdog(app.clone(), Arc::clone(rt), audio.clone(), sid);
        }
    } else {
        audio::stop_shared_pipeline(audio);
    }

    Ok(())
}

fn dismiss_hud(app: &AppHandle, rt: &SharedHud) -> Result<(), String> {
    let mut s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))?;

    s.phase = HudPhase::Stopped;
    s.visible = false;
    s.session_id = s.session_id.wrapping_add(1);
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
            }
            HudPhase::Stopped => {
                s.visible = false;
                s.session_id = s.session_id.wrapping_add(1);
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let hud_state: SharedHud = Arc::new(Mutex::new(HudRuntime::default()));
    // cpal stream is !Send; SharedAudioPipeline uses unsafe Send/Sync — see audio/mod.rs
    #[allow(clippy::arc_with_non_send_sync)]
    let audio_pipeline = SharedAudioPipeline(Arc::new(Mutex::new(None)));
    let is_paused = Arc::new(AtomicBool::new(false));

    tauri::Builder::default()
        .manage(Arc::clone(&hud_state))
        .manage(audio_pipeline.clone())
        .manage(Arc::clone(&is_paused))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .setup({
            let hud_state = Arc::clone(&hud_state);
            let audio_for_shortcut = audio_pipeline.clone();
            let is_paused_for_shortcut = Arc::clone(&is_paused);
            move |app| {
                let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                db::init_db(&dir.join("jarvis.db")).map_err(|e| e.to_string())?;

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
                        let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload()) else {
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
                                .with_shortcuts(["ctrl+shift+j"])
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
            hud_dismiss
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

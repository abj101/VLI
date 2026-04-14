mod audio;
mod commands;
mod db;
mod hud;

use audio::SharedAudioPipeline;
use hud::{sync_hud_window, HudPhase, HUD_WINDOW_LABEL};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{Builder as ShortcutBuilder, ShortcutState};

#[derive(Debug, Default)]
struct HudRuntime {
    phase: HudPhase,
    visible: bool,
}

type SharedHud = Arc<Mutex<HudRuntime>>;

fn stop_listening_audio(slot: &SharedAudioPipeline) {
    *slot.lock().unwrap() = None;
}

fn try_start_listening_audio(app: &AppHandle, slot: &SharedAudioPipeline) {
    let mut g = slot.lock().unwrap();
    *g = None;
    match audio::AudioPipeline::start(app) {
        Ok(p) => *g = Some(p),
        Err(msg) => {
            let _ = app.emit(
                "audio-error",
                serde_json::json!({ "message": msg }),
            );
        }
    }
}

fn emit_hud_phase(app: &AppHandle, phase: HudPhase) {
    let _ = app.emit(
        "hud-phase",
        serde_json::json!({ "phase": phase.as_str() }),
    );
}

fn show_hud_from_hotkey(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
) -> Result<(), String> {
    let mut s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))?;

    if !s.visible {
        s.visible = true;
        s.phase = HudPhase::Listening;
        window
            .center()
            .map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    } else {
        s.phase = HudPhase::Stopped;
        s.visible = false;
        window.hide().map_err(|e| e.to_string())?;
    }

    let phase = s.phase;
    drop(s);

    sync_hud_window(app, phase)?;
    emit_hud_phase(app, phase);

    if phase == HudPhase::Listening {
        try_start_listening_audio(app, audio);
    } else {
        stop_listening_audio(audio);
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
fn hud_set_phase(phase: HudPhase, app: AppHandle, state: State<'_, SharedHud>) -> Result<(), String> {
    {
        let mut s = state.lock().map_err(|_| "hud state poisoned".to_string())?;
        s.phase = phase;
    }
    let phase = state
        .lock()
        .map_err(|_| "hud state poisoned".to_string())?
        .phase;
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
    dismiss_hud(&app, &*state)?;
    stop_listening_audio(&*audio);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let hud_state: SharedHud = Arc::new(Mutex::new(HudRuntime::default()));
    let audio_pipeline: SharedAudioPipeline = Arc::new(Mutex::new(None));

    tauri::Builder::default()
        .manage(Arc::clone(&hud_state))
        .manage(Arc::clone(&audio_pipeline))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .setup({
            let hud_state = Arc::clone(&hud_state);
            let audio_for_shortcut = Arc::clone(&audio_pipeline);
            move |app| {
                let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                db::init_db(&dir.join("jarvis.db")).map_err(|e| e.to_string())?;

                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                {
                    app.handle()
                        .plugin(
                            ShortcutBuilder::new()
                                .with_shortcuts(["ctrl+shift+j"])
                                .map_err(|e| e.to_string())?
                                .with_handler({
                                    let hud_state = Arc::clone(&hud_state);
                                    let audio_for_shortcut = Arc::clone(&audio_for_shortcut);
                                    move |app, _shortcut, event| {
                                        if event.state != ShortcutState::Pressed {
                                            return;
                                        }
                                        let _ = show_hud_from_hotkey(app, &hud_state, &audio_for_shortcut);
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

//! Voice dictation: stream STT into the focused text field without HUD focus steal.

pub mod cleanup;
mod session;

use crate::audio::{self, stt::TranscriptUpdate, SharedAudioPipeline};
use serde::Serialize;
pub use session::{DictationSessionController, HudSnapshot};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

pub struct DictationState {
    controller: Mutex<DictationSessionController>,
}

impl Default for DictationState {
    fn default() -> Self {
        Self {
            controller: Mutex::new(DictationSessionController::default()),
        }
    }
}

impl DictationState {
    pub fn is_active(&self) -> bool {
        self.controller
            .lock()
            .map(|g| g.is_active())
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationPhasePayload {
    pub active: bool,
    pub session_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

fn emit_dictation_phase(app: &AppHandle, active: bool, session_id: u64, text: Option<String>) {
    let _ = app.emit(
        "dictation-phase",
        DictationPhasePayload {
            active,
            session_id,
            text,
        },
    );
}

fn hud_snapshot(hud: &crate::SharedHud) -> Result<HudSnapshot, String> {
    let s = hud.lock().map_err(|_| "hud state poisoned".to_string())?;
    Ok(HudSnapshot {
        visible: s.visible,
        phase: s.phase,
        overlay_mode: s.overlay_mode,
    })
}

fn with_controller<F, T>(app: &AppHandle, f: F) -> Result<T, String>
where
    F: FnOnce(&mut DictationSessionController) -> Result<T, String>,
{
    let state = app.state::<DictationState>();
    let mut guard = state
        .controller
        .lock()
        .map_err(|_| "dictation state poisoned".to_string())?;
    f(&mut guard)
}

pub fn is_active(app: &AppHandle) -> bool {
    app.try_state::<DictationState>()
        .is_some_and(|s| s.is_active())
}

pub fn stop_dictation(
    app: &AppHandle,
    hud: &crate::SharedHud,
    audio: &SharedAudioPipeline,
) -> Result<(), String> {
    audio::finish_shared_pipeline(app, audio);
    let session_id = with_controller(app, |ctrl| Ok(ctrl.stop()))?;
    crate::hide_dictation_overlay(app, hud)?;
    emit_dictation_phase(app, false, session_id, None);
    Ok(())
}

pub fn start_dictation(
    app: &AppHandle,
    hud: &crate::SharedHud,
    audio: &SharedAudioPipeline,
) -> Result<u64, String> {
    let focused = crate::window::focused_window();
    audio::stop_shared_pipeline(app, audio);
    let snapshot = hud_snapshot(hud)?;
    if snapshot.blocks_dictation() {
        crate::abort_command_hud_session(app, hud, audio)?;
        if hud_snapshot(hud)?.blocks_dictation() {
            return Err("cannot start dictation while voice HUD is active".into());
        }
    }

    let session_id = {
        let mut s = hud.lock().map_err(|_| "hud state poisoned".to_string())?;
        let snapshot = HudSnapshot {
            visible: s.visible,
            phase: s.phase,
            overlay_mode: s.overlay_mode,
        };
        let session_id = s.session_id.wrapping_add(1);
        with_controller(app, |ctrl| {
            ctrl.start(session_id, snapshot)?;
            ctrl.set_context(cleanup::context_for_exe(
                focused
                    .as_ref()
                    .map(|window| window.exe_path.as_str())
                    .unwrap_or(""),
            ));
            Ok(())
        })?;
        s.session_id = session_id;
        session_id
    };

    if let Some(suppressed) = app.try_state::<audio::WakeMicSuppressed>() {
        use std::sync::atomic::Ordering;
        suppressed.0.store(true, Ordering::SeqCst);
    }

    crate::try_start_listening_audio(app, audio, session_id, false, true);
    crate::show_dictation_overlay(app, hud, session_id)?;
    emit_dictation_phase(app, true, session_id, None);
    Ok(session_id)
}

pub fn toggle_dictation(
    app: &AppHandle,
    hud: &crate::SharedHud,
    audio: &SharedAudioPipeline,
) -> Result<(), String> {
    if is_active(app) {
        stop_dictation(app, hud, audio)
    } else {
        start_dictation(app, hud, audio)?;
        Ok(())
    }
}

pub fn dictation_prompt(app: &AppHandle, session_id: u64) -> String {
    let tail = with_controller(app, |ctrl| {
        Ok(if ctrl.accepts(session_id) {
            ctrl.tail().to_string()
        } else {
            String::new()
        })
    })
    .unwrap_or_default();
    let vocabulary = crate::open_db_connection(app)
        .ok()
        .and_then(|conn| crate::db::get_app_settings(&conn).ok())
        .map(|s| s.dictation_vocabulary)
        .unwrap_or_default();
    format_prompt(&vocabulary, &tail)
}

fn format_prompt(vocabulary: &str, tail: &str) -> String {
    let words = vocabulary
        .split(',')
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    if words.is_empty() {
        tail.to_string()
    } else if tail.is_empty() {
        format!("Vocabulary: {words}.")
    } else {
        format!("Vocabulary: {words}. {tail}")
    }
}

/// Dispatch directly for shutdown ordering, then notify webviews without re-entering the native listener.
pub fn publish_transcript(
    app: &AppHandle,
    update: TranscriptUpdate,
    decode_ms: u128,
) -> Result<(), String> {
    handle_transcript_timed(app, &update, decode_ms)?;
    app.emit_filter("transcript-update", update, |target| {
        !matches!(target, tauri::EventTarget::App)
    })
    .map_err(|e| e.to_string())
}

pub fn handle_transcript(app: &AppHandle, update: &TranscriptUpdate) -> Result<(), String> {
    handle_transcript_timed(app, update, 0)
}

fn handle_transcript_timed(
    app: &AppHandle,
    update: &TranscriptUpdate,
    decode_ms: u128,
) -> Result<(), String> {
    let (accepted, context) = with_controller(app, |ctrl| {
        Ok((ctrl.accepts(update.hud_session_id), ctrl.context()))
    })?;
    if !accepted {
        return Ok(());
    }
    let cleanup_started = std::time::Instant::now();
    let mut text = update.text.clone();
    if update.is_final {
        if let Ok(settings) = crate::open_db_connection(app)
            .and_then(|conn| crate::db::get_app_settings(&conn).map_err(|e| e.to_string()))
        {
            if settings.dictation_cleanup {
                text = cleanup::cleanup(
                    &text,
                    &settings.dictation_style,
                    context,
                    settings.local_whisper_use_gpu,
                );
            }
        }
    }
    let cleanup_ms = cleanup_started.elapsed().as_millis();
    if update.is_final {
        text = with_controller(app, |ctrl| {
            Ok(ctrl
                .final_text(update.hud_session_id, &text, true)
                .unwrap_or_default())
        })?;
    }
    let paste_started = std::time::Instant::now();
    if !update.is_final {
        emit_dictation_phase(app, true, update.hud_session_id, Some(text.clone()));
    }
    if dispatch_final(update.is_final, &text, &mut |text| {
        let previous = crate::commands::executor::get_clipboard_text().ok();
        let result = crate::commands::executor::set_clipboard_text(text)
            .and_then(|_| crate::input::send_keys::send_keys("^v"));
        if result.is_ok() {
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        if let Some(previous) = previous {
            if previous.trim().is_empty() {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = clipboard.set_text(previous);
                }
            } else {
                let _ = crate::commands::executor::set_clipboard_text(&previous);
            }
        }
        result.or_else(|_| crate::input::type_text::type_unicode(text))
    })? {
        with_controller(app, |ctrl| {
            if ctrl.accepts(update.hud_session_id) {
                ctrl.remember(&text);
            }
            Ok(())
        })?;
        emit_dictation_phase(app, true, update.hud_session_id, None);
    }
    if update.is_final {
        log::debug!(
            "dictation: decode {decode_ms} ms, cleanup {cleanup_ms} ms, paste {} ms",
            paste_started.elapsed().as_millis()
        );
    }
    Ok(())
}

fn dispatch_final(
    is_final: bool,
    text: &str,
    insert: &mut impl FnMut(&str) -> Result<(), String>,
) -> Result<bool, String> {
    if !is_final || text.is_empty() {
        return Ok(false);
    }
    insert(text)?;
    Ok(true)
}

/// Stop dictation when HUD opens for command listening.
pub fn stop_for_hud(app: &AppHandle, hud: &crate::SharedHud, audio: &SharedAudioPipeline) {
    if is_active(app) {
        let _ = stop_dictation(app, hud, audio);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prompt_combines_vocabulary_and_tail() {
        assert_eq!(
            format_prompt("Claude, Jarvis, Tauri", ""),
            "Vocabulary: Claude, Jarvis, Tauri."
        );
        assert_eq!(format_prompt("", "Hello."), "Hello.");
        assert_eq!(
            format_prompt("Claude", "Hello."),
            "Vocabulary: Claude. Hello."
        );
    }
    #[test]
    fn partials_never_insert_final_inserts_once() {
        let mut calls = Vec::new();
        let mut insert = |text: &str| {
            calls.push(text.to_owned());
            Ok(())
        };
        dispatch_final(false, "hel", &mut insert).unwrap();
        dispatch_final(false, "hello world", &mut insert).unwrap();
        dispatch_final(true, "hello world", &mut insert).unwrap();
        assert_eq!(calls, vec!["hello world"]);
    }
}


//! Voice dictation: stream STT into the focused text field without HUD focus steal.

mod injector;
mod reconcile;
mod screen;
mod session;

pub use injector::SystemTextInjector;
pub use reconcile::TranscriptReconciler;
pub use session::{DictationSessionController, HudSnapshot};
use crate::audio::{self, stt::TranscriptUpdate, SharedAudioPipeline};
use serde::Serialize;
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
    let mut injector = SystemTextInjector;
    let session_id = with_controller(app, |ctrl| ctrl.stop(&mut injector))?;

    audio::stop_shared_pipeline(app, audio);
    crate::hide_dictation_overlay(app, hud)?;
    emit_dictation_phase(app, false, session_id, None);
    Ok(())
}

pub fn start_dictation(
    app: &AppHandle,
    hud: &crate::SharedHud,
    audio: &SharedAudioPipeline,
) -> Result<u64, String> {
    let snapshot = hud_snapshot(hud)?;
    let session_id = with_controller(app, |ctrl| ctrl.start(snapshot))?;

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

pub fn handle_transcript(app: &AppHandle, update: &TranscriptUpdate) -> Result<(), String> {
    let mut injector = SystemTextInjector;
    let Some(text_preview) = with_controller(app, |ctrl| {
        ctrl.on_transcript(
            update.hud_session_id,
            &update.text,
            update.kind,
            &mut injector,
        )
    })?
    else {
        return Ok(());
    };

    emit_dictation_phase(app, true, update.hud_session_id, text_preview);
    Ok(())
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
    use crate::dictation::injector::{apply_transcript, apply_transcript_segment_with_kind, MockTextInjector};

    #[test]
    fn apply_stt_delta_matches_reconcile_module() {
        let delta = reconcile::apply_stt_delta("hi", "hi there");
        assert_eq!(delta.suffix, " there");
    }

    #[test]
    fn silence_reset_kind_clears_segment_without_typing() {
        let mut segment = "hello world".to_string();
        let mut last = "hello world".to_string();
        let mut session = "hello world".to_string();
        let mut typed = true;
        let mut mock = MockTextInjector::new();

        apply_transcript_segment_with_kind(
            &mut segment,
            &mut last,
            &mut session,
            &mut typed,
            "",
            crate::audio::transcript_event::TranscriptPartialKind::SilenceReset,
            &mut mock,
        )
        .unwrap();

        assert!(mock.type_calls.is_empty());
        assert!(mock.backspace_calls.is_empty());
        assert_eq!(segment, "hello world");
        assert!(last.is_empty());
    }

    #[test]
    fn handle_transcript_applies_delta_to_injector() {
        let mut reconciler = TranscriptReconciler::new();
        let mut mock = MockTextInjector::new();

        apply_transcript(&mut reconciler, "hello", &mut mock).unwrap();
        apply_transcript(&mut reconciler, "hello world", &mut mock).unwrap();

        assert_eq!(mock.type_calls, vec!["hello", " world"]);
        assert_eq!(reconciler.segment_injected(), "hello world");
    }
}

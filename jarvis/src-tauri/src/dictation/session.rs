//! Dictation session lifecycle — start/stop/toggle/flush/transcript without Tauri.

use super::injector::{apply_transcript, apply_transcript_with_kind, TextInjector};
use super::reconcile::TranscriptReconciler;
use crate::audio::transcript_event::TranscriptPartialKind;
use crate::hud::{HudOverlayMode, HudPhase};

#[derive(Debug, Clone)]
pub struct DictationSession {
    pub session_id: u64,
    pub reconciler: TranscriptReconciler,
}

impl DictationSession {
    pub fn new(session_id: u64) -> Self {
        Self {
            session_id,
            reconciler: TranscriptReconciler::new(),
        }
    }
}

/// HUD visibility + phase snapshot for collision checks (no `SharedHud` lock in tests).
#[derive(Debug, Clone, Copy)]
pub struct HudSnapshot {
    pub visible: bool,
    pub phase: HudPhase,
    pub overlay_mode: HudOverlayMode,
}

impl HudSnapshot {
    pub fn blocks_dictation(self) -> bool {
        self.visible
            && self.overlay_mode == HudOverlayMode::Command
            && matches!(
                self.phase,
                HudPhase::Listening
                    | HudPhase::Matched
                    | HudPhase::Routing
                    | HudPhase::Executing
                    | HudPhase::AwaitingInput
            )
    }
}

/// Owns session id allocation and active reconcile state.
#[derive(Debug)]
pub struct DictationSessionController {
    session: Option<DictationSession>,
    next_session_id: u64,
}

impl Default for DictationSessionController {
    fn default() -> Self {
        Self {
            session: None,
            next_session_id: 1,
        }
    }
}

impl DictationSessionController {
    pub fn is_active(&self) -> bool {
        self.session.is_some()
    }

    pub fn start(&mut self, hud: HudSnapshot) -> Result<u64, String> {
        if self.is_active() {
            return Err("dictation already active".into());
        }
        if hud.blocks_dictation() {
            return Err("cannot start dictation while voice HUD is active".into());
        }

        let session_id = self.next_session_id;
        self.next_session_id = self.next_session_id.saturating_add(1);
        self.session = Some(DictationSession::new(session_id));
        Ok(session_id)
    }

    /// Flush pending transcript, clear session; returns stopped session id (0 if none).
    pub fn stop<I: TextInjector>(&mut self, injector: &mut I) -> Result<u64, String> {
        if let Some(session) = self.session.as_mut() {
            flush_session(session, injector)?;
        }
        let id = self.session.as_ref().map(|s| s.session_id).unwrap_or(0);
        self.session = None;
        Ok(id)
    }

    /// Apply one STT update.
    ///
    /// - `Ok(None)` — stale `session_id`, no apply
    /// - `Ok(Some(preview))` — applied; `preview` is `None` when STT text was empty
    pub fn on_transcript<I: TextInjector>(
        &mut self,
        session_id: u64,
        text: &str,
        kind: TranscriptPartialKind,
        injector: &mut I,
    ) -> Result<Option<Option<String>>, String> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| "dictation not active".to_string())?;

        if session_id != session.session_id {
            return Ok(None);
        }

        apply_transcript_with_kind(&mut session.reconciler, text, kind, injector)?;

        Ok(Some(
            if matches!(kind, TranscriptPartialKind::SilenceReset) || text.is_empty() {
                None
            } else {
                Some(text.to_string())
            },
        ))
    }
}

#[cfg(test)]
impl DictationSessionController {
    pub fn current_session_id(&self) -> Option<u64> {
        self.session.as_ref().map(|s| s.session_id)
    }

    pub fn toggle<I: TextInjector>(
        &mut self,
        hud: HudSnapshot,
        injector: &mut I,
    ) -> Result<Option<u64>, String> {
        if self.is_active() {
            self.stop(injector)?;
            Ok(None)
        } else {
            Ok(Some(self.start(hud)?))
        }
    }

    pub fn flush<I: TextInjector>(&mut self, injector: &mut I) -> Result<(), String> {
        if let Some(session) = self.session.as_mut() {
            flush_session(session, injector)?;
        }
        Ok(())
    }
}

fn flush_session<I: TextInjector>(
    session: &mut DictationSession,
    injector: &mut I,
) -> Result<(), String> {
    let last = session.reconciler.last_stt();
    if last.is_empty() {
        return Ok(());
    }
    if session.reconciler.segment_injected() == last {
        return Ok(());
    }
    let pending = last.to_string();
    apply_transcript(&mut session.reconciler, &pending, injector)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictation::injector::MockTextInjector;
    use crate::audio::transcript_event::TranscriptPartialKind;
    use crate::hud::{HudOverlayMode, HudPhase};

    #[test]
    fn start_sets_active_session() {
        let mut ctrl = DictationSessionController::default();
        assert!(!ctrl.is_active());

        let session_id = ctrl
            .start(HudSnapshot {
                visible: false,
                phase: HudPhase::Idle,
                overlay_mode: HudOverlayMode::Command,
            })
            .unwrap();

        assert!(ctrl.is_active());
        assert_eq!(ctrl.current_session_id(), Some(session_id));
    }

    #[test]
    fn stop_clears_session() {
        let mut ctrl = DictationSessionController::default();
        let mut injector = MockTextInjector::new();
        let session_id = ctrl
            .start(HudSnapshot {
                visible: false,
                phase: HudPhase::Idle,
                overlay_mode: HudOverlayMode::Command,
            })
            .unwrap();
        assert!(ctrl.is_active());

        let stopped = ctrl.stop(&mut injector).unwrap();
        assert_eq!(stopped, session_id);
        assert!(!ctrl.is_active());
        assert_eq!(ctrl.current_session_id(), None);
    }

    #[test]
    fn flush_applies_pending_transcript() {
        let mut ctrl = DictationSessionController::default();
        let mut injector = MockTextInjector::new();
        ctrl.start(HudSnapshot {
            visible: false,
            phase: HudPhase::Idle,
            overlay_mode: HudOverlayMode::Command,
        })
        .unwrap();

        {
            let session = ctrl.session.as_mut().unwrap();
            session.reconciler = TranscriptReconciler::from_parts(
                "hello".into(),
                "hello world".into(),
                "hello".into(),
                true,
            );
        }

        ctrl.flush(&mut injector).unwrap();
        assert_eq!(
            ctrl.session.as_ref().unwrap().reconciler.segment_injected(),
            "hello world"
        );
    }

    #[test]
    fn flush_is_noop_when_already_synced() {
        let mut ctrl = DictationSessionController::default();
        let mut injector = MockTextInjector::new();
        ctrl.start(HudSnapshot {
            visible: false,
            phase: HudPhase::Idle,
            overlay_mode: HudOverlayMode::Command,
        })
        .unwrap();

        {
            let session = ctrl.session.as_mut().unwrap();
            session.reconciler = TranscriptReconciler::from_parts(
                "hello world".into(),
                "hello world".into(),
                "hello world".into(),
                true,
            );
        }

        ctrl.flush(&mut injector).unwrap();
        assert_eq!(
            ctrl.session.as_ref().unwrap().reconciler.segment_injected(),
            "hello world"
        );
        assert!(injector.type_calls.is_empty());
    }

    #[test]
    fn hud_active_rejects_start() {
        let mut ctrl = DictationSessionController::default();
        let err = ctrl
            .start(HudSnapshot {
                visible: true,
                phase: HudPhase::Listening,
                overlay_mode: HudOverlayMode::Command,
            })
            .unwrap_err();
        assert_eq!(err, "cannot start dictation while voice HUD is active");
        assert!(!ctrl.is_active());
    }

    #[test]
    fn hud_snapshot_blocks_active_listen_phases_only() {
        assert!(HudSnapshot {
            visible: true,
            phase: HudPhase::Listening,
            overlay_mode: HudOverlayMode::Command,
        }
        .blocks_dictation());
        assert!(!HudSnapshot {
            visible: true,
            phase: HudPhase::Listening,
            overlay_mode: HudOverlayMode::Dictation,
        }
        .blocks_dictation());
        assert!(!HudSnapshot {
            visible: true,
            phase: HudPhase::Idle,
            overlay_mode: HudOverlayMode::Command,
        }
        .blocks_dictation());
        assert!(!HudSnapshot {
            visible: false,
            phase: HudPhase::Listening,
            overlay_mode: HudOverlayMode::Command,
        }
        .blocks_dictation());
    }

    #[test]
    fn on_transcript_applies_delta_to_injector() {
        let mut ctrl = DictationSessionController::default();
        let mut injector = MockTextInjector::new();
        let session_id = ctrl
            .start(HudSnapshot {
                visible: false,
                phase: HudPhase::Idle,
                overlay_mode: HudOverlayMode::Command,
            })
            .unwrap();

        {
            let session = ctrl.session.as_mut().unwrap();
            session.reconciler = TranscriptReconciler::from_parts(
                "hello".into(),
                "hello".into(),
                "hello".into(),
                true,
            );
        }

        let preview = ctrl
            .on_transcript(
                session_id,
                "hello world",
                TranscriptPartialKind::Growth,
                &mut injector,
            )
            .unwrap();
        assert_eq!(preview, Some(Some("hello world".to_string())));
        assert_eq!(injector.type_calls, vec![" world"]);
    }

    #[test]
    fn toggle_starts_then_stops() {
        let mut ctrl = DictationSessionController::default();
        let mut injector = MockTextInjector::new();
        let hud = HudSnapshot {
            visible: false,
            phase: HudPhase::Idle,
            overlay_mode: HudOverlayMode::Command,
        };

        assert_eq!(ctrl.toggle(hud, &mut injector).unwrap(), Some(1));
        assert!(ctrl.is_active());
        assert_eq!(ctrl.toggle(hud, &mut injector).unwrap(), None);
        assert!(!ctrl.is_active());
    }
}

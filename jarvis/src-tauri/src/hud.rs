//! HUD window phase + click-through policy (Task 3).

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use tauri::AppHandle;
use tauri::Manager;

pub const HUD_WINDOW_LABEL: &str = "hud";

/// Mirrors `HudPhase` in `jarvis/src/types.ts` (snake_case strings on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudPhase {
    Idle,
    Listening,
    Matched,
    Executing,
    AwaitingInput,
    Done,
    Stopped,
}

impl Default for HudPhase {
    fn default() -> Self {
        Self::Idle
    }
}

impl HudPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Listening => "listening",
            Self::Matched => "matched",
            Self::Executing => "executing",
            Self::AwaitingInput => "awaiting_input",
            Self::Done => "done",
            Self::Stopped => "stopped",
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "idle" => Ok(Self::Idle),
            "listening" => Ok(Self::Listening),
            "matched" => Ok(Self::Matched),
            "executing" => Ok(Self::Executing),
            "awaiting_input" => Ok(Self::AwaitingInput),
            "done" => Ok(Self::Done),
            "stopped" => Ok(Self::Stopped),
            other => Err(format!("unknown hud phase `{other}`")),
        }
    }
}

impl Serialize for HudPhase {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HudPhase {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PhaseVisitor;

        impl Visitor<'_> for PhaseVisitor {
            type Value = HudPhase;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a snake_case hud phase string")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                HudPhase::parse(v).map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_str(PhaseVisitor)
    }
}

/// `true` → window ignores mouse (clicks pass through to desktop).
pub fn ignore_cursor_for_phase(phase: HudPhase) -> bool {
    matches!(
        phase,
        HudPhase::Idle | HudPhase::Done | HudPhase::Stopped
    )
}

pub fn sync_hud_window(app: &AppHandle, phase: HudPhase) -> Result<(), String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| format!("missing webview window `{HUD_WINDOW_LABEL}`"))?;
    window
        .set_ignore_cursor_events(ignore_cursor_for_phase(phase))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{ignore_cursor_for_phase, HudPhase};

    #[test]
    fn click_through_when_idle_done_stopped() {
        assert!(ignore_cursor_for_phase(HudPhase::Idle));
        assert!(ignore_cursor_for_phase(HudPhase::Done));
        assert!(ignore_cursor_for_phase(HudPhase::Stopped));
    }

    #[test]
    fn interactive_when_listening_executing_matched_awaiting_input() {
        assert!(!ignore_cursor_for_phase(HudPhase::Listening));
        assert!(!ignore_cursor_for_phase(HudPhase::Executing));
        assert!(!ignore_cursor_for_phase(HudPhase::Matched));
        assert!(!ignore_cursor_for_phase(HudPhase::AwaitingInput));
    }

    #[test]
    fn phase_round_trip_strings() {
        for p in [
            HudPhase::Idle,
            HudPhase::Listening,
            HudPhase::Matched,
            HudPhase::Executing,
            HudPhase::AwaitingInput,
            HudPhase::Done,
            HudPhase::Stopped,
        ] {
            assert_eq!(HudPhase::parse(p.as_str()).unwrap(), p);
        }
    }
}

import type {
  ActionErrorPayload,
  ActionStatus,
  AudioErrorPayload,
  DictationPhasePayload,
  HudOverlayMode,
  HudPhase,
  HudPhasePayload,
  MatchResult,
  TranscriptUpdate,
} from "../types";

/** Ignore sub-threshold mic level jitter so idle waveform does not re-render every tick. */
const AMPLITUDE_UI_EPSILON = 0.02;

export type HudWireTopic =
  | "hud-phase"
  | "transcript-update"
  | "match-result"
  | "action-status"
  | "action-error"
  | "amplitude-update"
  | "audio-error"
  | "dictation-phase";

export type HudState = {
  phase: HudPhase;
  /** Authoritative session from Rust `hud-phase` events; filters stale STT. */
  sessionId: number;
  overlayMode: HudOverlayMode;
  transcript: string;
  transcriptFinal: boolean;
  match: MatchResult | null;
  actionText: string | null;
  actionError: string | null;
  amplitude: number;
  audioError: string | null;
  /** Last sentence inserted by dictation; `id` bumps per insert so repeats replay the print. */
  dictationPrint: { text: string; id: number };
  dictationSessionId: number;
  dictationWaiting: boolean;
};

export const initialHudState: HudState = {
  phase: "idle",
  sessionId: 0,
  overlayMode: "command",
  transcript: "",
  transcriptFinal: false,
  match: null,
  actionText: null,
  actionError: null,
  amplitude: 0,
  audioError: null,
  dictationPrint: { text: "", id: 0 },
  dictationSessionId: 0,
  dictationWaiting: false,
};

export function reduceHudState(
  state: HudState,
  topic: HudWireTopic,
  payload:
    | HudPhasePayload
    | TranscriptUpdate
    | MatchResult
    | ActionStatus
    | ActionErrorPayload
    | { amplitude: number }
    | AudioErrorPayload
    | DictationPhasePayload,
): HudState {
  switch (topic) {
    case "hud-phase": {
      const { phase, session_id: sessionId, overlay_mode: overlayMode } =
        payload as HudPhasePayload;
      if (sessionId != null && sessionId < state.sessionId) {
        return state;
      }
      const next: HudState = {
        ...state,
        phase,
        sessionId: sessionId ?? state.sessionId,
        overlayMode: overlayMode ?? state.overlayMode,
      };
      if (phase === "listening") {
        next.transcript = "";
        next.transcriptFinal = false;
        next.match = null;
        next.actionText = null;
        next.actionError = null;
        next.amplitude = 0;
        next.audioError = null;
      }
      if (phase === "awaiting_input") {
        // Drop previous command highlight so follow-up prompt/transcript can render.
        next.match = null;
        next.transcript = "";
        next.transcriptFinal = false;
      }
      if (
        phase === "routing" ||
        phase === "executing" ||
        phase === "done" ||
        phase === "stopped"
      ) {
        next.match = null;
        next.actionText = null;
      if (phase !== "done") next.actionError = null;
        next.transcript = "";
        next.transcriptFinal = false;
        next.amplitude = 0;
      }
      return next;
    }
    case "transcript-update": {
      const u = payload as TranscriptUpdate;
      if (
        u.hud_session_id != null &&
        u.hud_session_id !== state.sessionId
      ) {
        return state;
      }
      if (state.overlayMode === "dictation") {
        return state;
      }
      // A decoder buffer reset is not a request to erase the phrase being read.
      if (u.kind === "silence_reset") {
        return state;
      }
      return {
        ...state,
        // Wake preroll is included in STT, but only the spoken command belongs in the HUD.
        transcript: u.text.replace(
          /^\s*(?:(?:hey|hay|hi|ok|okay)[\s,.:;!?-]+)?(?:jarvis|javis|jervis)\b[\s,.:;!?-]*/i,
          "",
        ),
        transcriptFinal: u.is_final,
        // Only clear match on new partials while still listening; late STT finals after match
        // would otherwise wipe the HUD before executing/done.
        match: state.phase === "listening" ? null : state.match,
      };
    }
    case "match-result":
      return { ...state, match: payload as MatchResult };
    case "action-status":
      // The command that starts dictation can finish after the dictation HUD opens.
      if (state.overlayMode === "dictation") return state;
      return { ...state, actionText: (payload as ActionStatus).text, actionError: null };
    case "action-error": {
      const { message } = payload as ActionErrorPayload;
      return { ...state, actionError: message };
    }
    case "amplitude-update": {
      const amp = (payload as { amplitude: number }).amplitude;
      const n = Number.isFinite(amp) ? amp : 0;
      const clamped = Math.max(0, Math.min(1, n));
      if (Math.abs(clamped - state.amplitude) < AMPLITUDE_UI_EPSILON) {
        return state;
      }
      return { ...state, amplitude: clamped };
    }
    case "audio-error": {
      const { message } = payload as AudioErrorPayload;
      return { ...state, audioError: message };
    }
    case "dictation-phase": {
      const update = payload as DictationPhasePayload;
      if (update.sessionId < state.dictationSessionId) return state;
      return {
        ...state,
        dictationSessionId: update.sessionId,
        dictationWaiting: update.active && (update.waitingForInput ?? state.dictationWaiting),
        dictationPrint: update.active && update.text
          ? { text: update.text, id: state.dictationPrint.id + 1 }
          : state.dictationPrint,
      };
    }
    default:
      return state;
  }
}

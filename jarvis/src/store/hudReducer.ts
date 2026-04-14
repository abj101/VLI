import type {
  ActionErrorPayload,
  ActionStatus,
  AudioErrorPayload,
  HudPhase,
  MatchResult,
  TranscriptUpdate,
} from "../types";

export type HudWireTopic =
  | "hud-phase"
  | "transcript-update"
  | "match-result"
  | "action-status"
  | "action-error"
  | "amplitude-update"
  | "audio-error";

export type HudState = {
  phase: HudPhase;
  transcript: string;
  transcriptFinal: boolean;
  match: MatchResult | null;
  actionText: string | null;
  actionError: string | null;
  amplitude: number;
  audioError: string | null;
};

export const initialHudState: HudState = {
  phase: "idle",
  transcript: "",
  transcriptFinal: false,
  match: null,
  actionText: null,
  actionError: null,
  amplitude: 0,
  audioError: null,
};

function clampSpan(textLen: number, spanStart: number, spanEnd: number) {
  const start = Math.max(0, Math.min(spanStart, textLen));
  const end = Math.max(start, Math.min(spanEnd, textLen));
  return { start, end };
}

export function sliceTranscriptBySpan(
  text: string,
  spanStart: number,
  spanEnd: number,
): { before: string; match: string; after: string } {
  const { start, end } = clampSpan(text.length, spanStart, spanEnd);
  return {
    before: text.slice(0, start),
    match: text.slice(start, end),
    after: text.slice(end),
  };
}

type HudPhasePayload = { phase: HudPhase };

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
    | AudioErrorPayload,
): HudState {
  switch (topic) {
    case "hud-phase": {
      const { phase } = payload as HudPhasePayload;
      const next: HudState = { ...state, phase };
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
      return next;
    }
    case "transcript-update": {
      const u = payload as TranscriptUpdate;
      return {
        ...state,
        transcript: u.text,
        transcriptFinal: u.is_final,
        // Only clear match on new partials while still listening; late STT finals after match
        // would otherwise wipe the HUD before executing/done.
        match: state.phase === "listening" ? null : state.match,
      };
    }
    case "match-result":
      return { ...state, match: payload as MatchResult };
    case "action-status":
      return { ...state, actionText: (payload as ActionStatus).text, actionError: null };
    case "action-error": {
      const { message } = payload as ActionErrorPayload;
      return { ...state, actionError: message };
    }
    case "amplitude-update": {
      const amp = (payload as { amplitude: number }).amplitude;
      const n = Number.isFinite(amp) ? amp : 0;
      return { ...state, amplitude: Math.max(0, Math.min(1, n)) };
    }
    case "audio-error": {
      const { message } = payload as AudioErrorPayload;
      return { ...state, audioError: message };
    }
    default:
      return state;
  }
}

import type {
  ActionStatus,
  HudPhase,
  MatchResult,
  TranscriptUpdate,
} from "../types";

export type HudWireTopic =
  | "hud-phase"
  | "transcript-update"
  | "match-result"
  | "action-status"
  | "amplitude-update";

export type HudState = {
  phase: HudPhase;
  transcript: string;
  transcriptFinal: boolean;
  match: MatchResult | null;
  actionText: string | null;
  amplitude: number;
};

export const initialHudState: HudState = {
  phase: "idle",
  transcript: "",
  transcriptFinal: false,
  match: null,
  actionText: null,
  amplitude: 0,
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
    | { amplitude: number },
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
        next.amplitude = 0;
      }
      return next;
    }
    case "transcript-update": {
      const u = payload as TranscriptUpdate;
      return {
        ...state,
        transcript: u.text,
        transcriptFinal: u.is_final,
        match: null,
      };
    }
    case "match-result":
      return { ...state, match: payload as MatchResult };
    case "action-status":
      return { ...state, actionText: (payload as ActionStatus).text };
    case "amplitude-update": {
      const amp = (payload as { amplitude: number }).amplitude;
      const n = Number.isFinite(amp) ? amp : 0;
      return { ...state, amplitude: Math.max(0, Math.min(1, n)) };
    }
    default:
      return state;
  }
}

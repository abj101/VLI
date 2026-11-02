import { create } from "zustand";
import {
  initialHudState,
  reduceHudState,
  type HudState,
  type HudWireTopic,
} from "./hudReducer";
import type {
  ActionErrorPayload,
  ActionStatus,
  AudioErrorPayload,
  DictationPhasePayload,
  HudPhasePayload,
  MatchResult,
  TranscriptUpdate,
} from "../types";

function pickHudState(s: HudStore): HudState {
  return {
    phase: s.phase,
    sessionId: s.sessionId,
    overlayMode: s.overlayMode,
    transcript: s.transcript,
    transcriptFinal: s.transcriptFinal,
    match: s.match,
    actionText: s.actionText,
    actionError: s.actionError,
    amplitude: s.amplitude,
    audioError: s.audioError,
    dictationPrint: s.dictationPrint,
    dictationSessionId: s.dictationSessionId,
    dictationWaiting: s.dictationWaiting,
  };
}

type HudStore = HudState & {
  applyIpc: (
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
  ) => void;
};

export const useHudStore = create<HudStore>((set) => ({
  ...initialHudState,
  applyIpc(topic, payload) {
    set((s) => ({
      ...s,
      ...reduceHudState(pickHudState(s), topic, payload),
    }));
  },
}));

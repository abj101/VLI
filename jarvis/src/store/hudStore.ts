import { create } from "zustand";
import {
  initialHudState,
  reduceHudState,
  type HudState,
  type HudWireTopic,
} from "./hudReducer";
import type {
  ActionStatus,
  HudPhase,
  MatchResult,
  TranscriptUpdate,
} from "../types";

function pickHudState(s: HudStore): HudState {
  return {
    phase: s.phase,
    transcript: s.transcript,
    transcriptFinal: s.transcriptFinal,
    match: s.match,
    actionText: s.actionText,
    amplitude: s.amplitude,
  };
}

export type HudStore = HudState & {
  applyIpc: (
    topic: HudWireTopic,
    payload:
      | { phase: HudPhase }
      | TranscriptUpdate
      | MatchResult
      | ActionStatus
      | { amplitude: number },
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

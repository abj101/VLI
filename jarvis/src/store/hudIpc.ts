import { listen } from "@tauri-apps/api/event";
import type {
  ActionErrorPayload,
  ActionStatus,
  AudioErrorPayload,
  HudPhase,
  MatchResult,
  TranscriptUpdate,
} from "../types";
import { useHudStore } from "./hudStore";

const ipcLog =
  import.meta.env.DEV
    ? (topic: string, detail: unknown) =>
        console.debug(`[jarvis:ipc] ${topic}`, detail)
    : () => {};

const HUD_PHASES = [
  "idle",
  "listening",
  "matched",
  "executing",
  "awaiting_input",
  "done",
  "stopped",
] as const satisfies readonly HudPhase[];

function isHudPhase(x: string): x is HudPhase {
  return (HUD_PHASES as readonly string[]).includes(x);
}

/** Subscribe to HUD-related Tauri events; returns unlisten-all. */
export async function subscribeHudIpc(): Promise<() => void> {
  const unsubs: Array<() => void> = [];

  const uPhase = await listen<{ phase: string }>("hud-phase", (e) => {
    ipcLog("hud-phase", e.payload);
    const p = e.payload.phase;
    if (isHudPhase(p)) {
      useHudStore.getState().applyIpc("hud-phase", { phase: p });
    }
  });
  unsubs.push(uPhase);

  const uTr = await listen<TranscriptUpdate>("transcript-update", (e) => {
    ipcLog("transcript-update", e.payload);
    useHudStore.getState().applyIpc("transcript-update", e.payload);
  });
  unsubs.push(uTr);

  const uMatch = await listen<MatchResult>("match-result", (e) => {
    ipcLog("match-result", e.payload);
    useHudStore.getState().applyIpc("match-result", e.payload);
  });
  unsubs.push(uMatch);

  const uAct = await listen<ActionStatus>("action-status", (e) => {
    ipcLog("action-status", e.payload);
    useHudStore.getState().applyIpc("action-status", e.payload);
  });
  unsubs.push(uAct);

  const uActErr = await listen<ActionErrorPayload>("action-error", (e) => {
    ipcLog("action-error", e.payload);
  });
  unsubs.push(uActErr);

  const uAmp = await listen<{ amplitude: number }>("amplitude-update", (e) => {
    useHudStore.getState().applyIpc("amplitude-update", e.payload);
  });
  unsubs.push(uAmp);

  const uAudErr = await listen<AudioErrorPayload>("audio-error", (e) => {
    ipcLog("audio-error", e.payload);
    useHudStore.getState().applyIpc("audio-error", e.payload);
  });
  unsubs.push(uAudErr);

  return () => {
    for (const u of unsubs) {
      u();
    }
  };
}

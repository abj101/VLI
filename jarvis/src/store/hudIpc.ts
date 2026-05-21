import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  ActionErrorPayload,
  ActionStatus,
  AudioErrorPayload,
  HudPhase,
  MatchResult,
  TranscriptUpdate,
  WakeDetectedPayload,
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

/** Pull authoritative phase after listeners attach (catches events emitted during webview load). */
async function applyHudPhaseFromRust(): Promise<void> {
  try {
    const p = await invoke<HudPhase>("hud_get_phase");
    if (isHudPhase(p)) {
      useHudStore.getState().applyIpc("hud-phase", { phase: p });
    }
  } catch {
    /* Web-only / tests without Tauri */
  }
}

/** Subscribe to HUD-related Tauri events; returns unlisten-all. */
export async function subscribeHudIpc(): Promise<() => void> {
  const [
    uPhase,
    uTr,
    uWake,
    uMatch,
    uAct,
    uActErr,
    uAmp,
    uAudErr,
  ] = await Promise.all([
    listen<{ phase: string; session_id?: number }>("hud-phase", (e) => {
      ipcLog("hud-phase", e.payload);
      const p = e.payload.phase;
      if (isHudPhase(p)) {
        useHudStore.getState().applyIpc("hud-phase", {
          phase: p,
          session_id: e.payload.session_id,
        });
      }
    }),
    listen<TranscriptUpdate>("transcript-update", (e) => {
      ipcLog("transcript-update", e.payload);
      useHudStore.getState().applyIpc("transcript-update", e.payload);
    }),
    listen<WakeDetectedPayload>("wake-detected", (e) => {
      ipcLog("wake-detected", e.payload);
    }),
    listen<MatchResult>("match-result", (e) => {
      ipcLog("match-result", e.payload);
      useHudStore.getState().applyIpc("match-result", e.payload);
    }),
    listen<ActionStatus>("action-status", (e) => {
      ipcLog("action-status", e.payload);
      useHudStore.getState().applyIpc("action-status", e.payload);
    }),
    listen<ActionErrorPayload>("action-error", (e) => {
      ipcLog("action-error", e.payload);
      useHudStore.getState().applyIpc("action-error", e.payload);
    }),
    listen<{ amplitude: number }>("amplitude-update", (e) => {
      useHudStore.getState().applyIpc("amplitude-update", e.payload);
    }),
    listen<AudioErrorPayload>("audio-error", (e) => {
      ipcLog("audio-error", e.payload);
      useHudStore.getState().applyIpc("audio-error", e.payload);
    }),
  ]);

  await applyHudPhaseFromRust();

  const unsubs = [
    uPhase,
    uTr,
    uWake,
    uMatch,
    uAct,
    uActErr,
    uAmp,
    uAudErr,
  ];

  return () => {
    for (const u of unsubs) {
      u();
    }
  };
}

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  ActionErrorPayload,
  ActionStatus,
  AudioErrorPayload,
  HudOverlayMode,
  HudPhase,
  HudPhaseSnapshot,
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
  "routing",
  "executing",
  "awaiting_input",
  "done",
  "stopped",
] as const satisfies readonly HudPhase[];

function isHudPhase(x: string): x is HudPhase {
  return (HUD_PHASES as readonly string[]).includes(x);
}

function isHudOverlayMode(x: string): x is HudOverlayMode {
  return x === "command" || x === "dictation";
}

/** Pull authoritative phase after listeners attach (catches events emitted during webview load). */
async function applyHudPhaseFromRust(): Promise<void> {
  try {
    const snapshot = await invoke<HudPhaseSnapshot>("hud_get_phase");
    if (isHudPhase(snapshot.phase)) {
      useHudStore.getState().applyIpc("hud-phase", {
        phase: snapshot.phase,
        session_id: snapshot.sessionId,
        overlay_mode: isHudOverlayMode(snapshot.overlayMode)
          ? snapshot.overlayMode
          : "command",
      });
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
    listen<{ phase: string; session_id?: number; overlay_mode?: string }>(
      "hud-phase",
      (e) => {
      ipcLog("hud-phase", e.payload);
      const p = e.payload.phase;
      if (isHudPhase(p)) {
        const overlayMode = e.payload.overlay_mode;
        useHudStore.getState().applyIpc("hud-phase", {
          phase: p,
          session_id: e.payload.session_id,
          overlay_mode:
            overlayMode != null && isHudOverlayMode(overlayMode)
              ? overlayMode
              : undefined,
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
      const phase = useHudStore.getState().phase;
      if (phase !== "listening") {
        return;
      }
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

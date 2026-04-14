/** IPC + HUD contract shared by Rust events and React store (Task 1). */

export type HudPhase =
  | "idle"
  | "listening"
  | "matched"
  | "executing"
  | "awaiting_input"
  | "done"
  | "stopped";

export interface TranscriptUpdate {
  text: string;
  is_final: boolean;
  /** Rust tags each pipeline with the HUD `session_id`; optional on the client. */
  hud_session_id?: number;
}

export interface MatchResult {
  node_id: string;
  matched_phrase: string;
  span_start: number;
  span_end: number;
}

export interface ActionStatus {
  text: string;
}

/** Mic level 0..1 from `amplitude-update` (Task 4a). */
export interface AmplitudeUpdate {
  amplitude: number;
}

/** Mic/STT failure from `audio-error` (e.g. missing Whisper weights). */
export interface AudioErrorPayload {
  message: string;
}

/** Action failure from `action-error` (validation or launch). */
export interface ActionErrorPayload {
  message: string;
}

/** Compile-time smoke: literals must satisfy exported shapes. */
const _ipcContract: {
  phase: HudPhase;
  transcript: TranscriptUpdate;
  match: MatchResult;
  action: ActionStatus;
  amplitude: AmplitudeUpdate;
  audioError: AudioErrorPayload;
  actionError: ActionErrorPayload;
} = {
  phase: "idle",
  transcript: { text: "", is_final: false },
  match: {
    node_id: "seed-1",
    matched_phrase: "open notepad",
    span_start: 0,
    span_end: 12,
  },
  action: { text: "Opening Notepad…" },
  amplitude: { amplitude: 0.35 },
  audioError: { message: "Whisper model missing" },
  actionError: { message: "launch failed" },
};
void _ipcContract;

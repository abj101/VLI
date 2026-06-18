/** IPC + HUD contract shared by Rust events and React store (Task 1). */

export type HudPhase =
  | "idle"
  | "listening"
  | "matched"
  | "routing"
  | "executing"
  | "awaiting_input"
  | "done"
  | "stopped";

export type HudOverlayMode = "command" | "dictation";

export interface HudPhasePayload {
  phase: HudPhase;
  /** Matches Rust `HudRuntime::session_id` for this phase transition. */
  session_id?: number;
  overlay_mode?: HudOverlayMode;
}

export interface HudPhaseSnapshot {
  phase: HudPhase;
  sessionId: number;
  overlayMode: HudOverlayMode;
}

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
  remainder: string;
}

export interface ActionStatus {
  text: string;
}

export type CommandAction =
  | { open_app: { name: string; path: string; placement?: string } }
  | { open_url: { url: string } }
  | { open_target: { target: string; placement?: string } }
  | { place_window: { zone: string; monitor?: string } }
  | { run_script: { script: string; args: string[] } }
  | { send_keys: { keys: string } }
  | { wait: { ms: number } }
  | { speak: { text: string } }
  | { sub_prompt: { prompt: string } }
  | { run_command: { command_id: number; input?: string } }
  | { read_file: { path: string } }
  | { http_get: { url: string } }
  | { get_clipboard: Record<string, never> }
  | { show_notification: { title: string; body: string } }
  | { text_trim: { text: string } }
  | { text_match: { pattern: string; text: string } }
  | { text_split: { delimiter: string; text: string } }
  | { text_combine: { separator: string } }
  | { set_clipboard: { text: string } }
  | { list_folder: { path: string } }
  | { write_file: { path: string; content: string } }
  | { get_file_metadata: { path: string } }
  | { screenshot: { path?: string } }
  | { device_info: Record<string, never> }
  | { start_dictation: Record<string, never> }
  | { stop_dictation: Record<string, never> }
  | {
      if_else: {
        condition: "text_contains" | "regex_match" | "text_is_empty";
        text: string;
        pattern: string;
        then_actions: ActionPayload[];
        else_actions: ActionPayload[];
      };
    };

export type ActionPayload = CommandAction;

export interface TestStepResult {
  index: number;
  action_kind: string;
  status: string;
  output_preview: string;
  error?: string | null;
}

export interface TestCommandResult {
  steps: TestStepResult[];
}

/** Editor-only row until the user picks a type; never sent to the backend. */
export type EditorPendingAction = { editor_pending: Record<string, never> };

export type FormActionPayload = CommandAction | EditorPendingAction;

export function editorPendingAction(): EditorPendingAction {
  return { editor_pending: {} };
}

export function isEditorPendingAction(a: FormActionPayload): a is EditorPendingAction {
  return "editor_pending" in a;
}

export type MatchMode = "phrase" | "prefix";

export interface CommandNodePayload {
  id: number;
  name: string;
  trigger_phrases: string[];
  actions: ActionPayload[];
  enabled: boolean;
  fuzzy_threshold_pct: number;
  match_mode?: MatchMode;
  created_at: string;
}

export interface ToolParameter {
  name: string;
  param_type: string;
  description?: string;
  required: boolean;
  enum_values?: string[];
}

export interface ToolDefinitionPayload {
  id: number;
  name: string;
  display_name: string;
  description: string;
  parameters: ToolParameter[];
  actions: ActionPayload[];
  enabled: boolean;
  builtin: boolean;
  created_at: string;
}

/** Editor / IPC create-update body (DB assigns `id` and `created_at`). */
export interface NewToolDefinitionPayload {
  name: string;
  display_name: string;
  description: string;
  parameters: ToolParameter[];
  actions: ActionPayload[];
  enabled: boolean;
  builtin?: boolean;
}

export type ComposerKind = "command" | "tool" | "both";

/** Result of `generate_automation` (command composer). */
export interface GenerateAutomationResult {
  kind: ComposerKind;
  command?: Omit<CommandNodePayload, "id" | "created_at">;
  tool?: NewToolDefinitionPayload;
  confidence: number;
  summary: string;
  warnings: string[];
}

/** Status from `composer_status`. */
export interface ComposerStatus {
  featureCompiled: boolean;
  compileBackend: string;
  runtimeAvailable: boolean;
  modelPresent: boolean;
  modelPath: string | null;
  composerEnabled: boolean;
  ready: boolean;
  loading: boolean;
  message: string | null;
}

export type ResolvedTargetPreview =
  | {
      app: { display_name: string; exe_path: string; placement?: string };
    }
  | { url: { url: string; placement?: string } }
  | {
      ambiguous: {
        query: string;
        app_candidates: { display_name: string; exe_path: string; score: number }[];
        url_candidate: { url: string; score: number };
        placement?: string;
      };
    };

export interface OpenTargetPreview {
  utterance: string;
  target: string;
  placement?: string | null;
  resolved: ResolvedTargetPreview;
}

/** Mic level 0..1 from `amplitude-update` (Task 4a). */
export interface AmplitudeUpdate {
  amplitude: number;
}

/** Dictation session phase from `dictation-phase`. */
export interface DictationPhasePayload {
  active: boolean;
  sessionId: number;
  text?: string;
}

/** Mic/STT failure from `audio-error` (e.g. missing Whisper weights). */
export interface AudioErrorPayload {
  message: string;
}

/** Action failure from `action-error` (validation or launch). */
export interface ActionErrorPayload {
  message: string;
}

/** Wake word hit from Rust (`audio/wake/thread.rs`); DevTools / HUD badge (Phase 4). */
export interface WakeDetectedPayload {
  backend: string;
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
  wakeDetected: WakeDetectedPayload;
} = {
  phase: "idle",
  transcript: { text: "", is_final: false },
  match: {
    node_id: "seed-1",
    matched_phrase: "open notepad",
    span_start: 0,
    span_end: 12,
    remainder: "",
  },
  action: { text: "Opening Notepad…" },
  amplitude: { amplitude: 0.35 },
  audioError: { message: "Whisper model missing" },
  actionError: { message: "launch failed" },
  wakeDetected: { backend: "oww" },
};
void _ipcContract;

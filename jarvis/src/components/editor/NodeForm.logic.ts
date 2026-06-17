import {
  isEditorPendingAction,
  type ActionPayload,
  type CommandNodePayload,
  type FormActionPayload,
} from "../../types";
import { defaultIfElseAction, validateIfElseFields } from "./ifElse.logic";

export type ActionKind =
  | "open_app"
  | "open_url"
  | "open_target"
  | "place_window"
  | "run_script"
  | "send_keys"
  | "speak"
  | "wait"
  | "sub_prompt"
  | "run_command"
  | "read_file"
  | "http_get"
  | "get_clipboard"
  | "show_notification"
  | "text_trim"
  | "text_match"
  | "text_split"
  | "text_combine"
  | "set_clipboard"
  | "list_folder"
  | "write_file"
  | "get_file_metadata"
  | "screenshot"
  | "device_info"
  | "if_else"
  | "pending";

/** Action kinds that map to a persisted `ActionPayload` (excludes UI-only `pending`). */
export type ConcreteActionKind = Exclude<ActionKind, "pending">;

export type FormModel = {
  id: number | null;
  triggerPhrases: string[];
  enabled: boolean;
  /** When true, words after the trigger become `{{remainder}}` for actions. */
  prefixMode: boolean;
  actions: FormActionPayload[];
};

export type FormErrors = {
  triggerPhrases?: string;
  actions?: string;
  /** Per-index messages for any action row (URLs, sub-prompt text, etc.). */
  actionErrors: Record<number, string>;
};

/** Stored `name` + IPC validation: first trigger phrase, max 72 chars (matches backend trim). */
export function derivedCommandName(triggerPhrases: string[]): string {
  const first = normalizeTriggerPhrases(triggerPhrases)[0] ?? "";
  return first.slice(0, 72);
}

export function defaultActionForKind(kind: ConcreteActionKind): ActionPayload {
  switch (kind) {
    case "open_app":
      return { open_app: { name: "", path: "" } };
    case "open_url":
      return { open_url: { url: "" } };
    case "open_target":
      return { open_target: { target: "{{remainder}}" } };
    case "place_window":
      return { place_window: { zone: "right_half" } };
    case "run_script":
      return { run_script: { script: "", args: [] } };
    case "send_keys":
      return { send_keys: { keys: "" } };
    case "speak":
      return { speak: { text: "" } };
    case "wait":
      return { wait: { ms: 0 } };
    case "sub_prompt":
      return { sub_prompt: { prompt: "" } };
    case "run_command":
      return { run_command: { command_id: 0, input: "" } };
    case "read_file":
      return { read_file: { path: "" } };
    case "http_get":
      return { http_get: { url: "" } };
    case "get_clipboard":
      return { get_clipboard: {} };
    case "show_notification":
      return { show_notification: { title: "", body: "" } };
    case "text_trim":
      return { text_trim: { text: "" } };
    case "text_match":
      return { text_match: { pattern: "", text: "" } };
    case "text_split":
      return { text_split: { delimiter: "", text: "" } };
    case "text_combine":
      return { text_combine: { separator: "" } };
    case "set_clipboard":
      return { set_clipboard: { text: "" } };
    case "list_folder":
      return { list_folder: { path: "" } };
    case "write_file":
      return { write_file: { path: "", content: "" } };
    case "get_file_metadata":
      return { get_file_metadata: { path: "" } };
    case "screenshot":
      return { screenshot: { path: "" } };
    case "device_info":
      return { device_info: {} };
    case "if_else":
      return defaultIfElseAction();
  }
}

export function emptyFormModel(): FormModel {
  return {
    id: null,
    triggerPhrases: [],
    enabled: true,
    prefixMode: false,
    actions: [],
  };
}

export function modelFromNode(node: CommandNodePayload | null): FormModel {
  if (!node) {
    return emptyFormModel();
  }
  return {
    id: node.id,
    triggerPhrases: [...node.trigger_phrases],
    enabled: node.enabled,
    prefixMode: node.match_mode === "prefix",
    actions: [...node.actions],
  };
}

/**
 * `fuzzy_threshold_pct: 0` means “use app default” (see Rust `resolve_fuzzy_threshold_pct`).
 * `name` is always derived from the first trigger phrase.
 */
export function toCommandPayload(model: FormModel): Omit<CommandNodePayload, "id" | "created_at"> {
  return {
    name: derivedCommandName(model.triggerPhrases),
    trigger_phrases: normalizeTriggerPhrases(model.triggerPhrases),
    actions: model.actions.filter((a): a is ActionPayload => !isEditorPendingAction(a)),
    enabled: model.enabled,
    fuzzy_threshold_pct: 0,
    match_mode: model.prefixMode ? "prefix" : "phrase",
  };
}

export function validateFormModel(model: FormModel): FormErrors {
  const errors: FormErrors = {
    actionErrors: {},
  };

  if (normalizeTriggerPhrases(model.triggerPhrases).length === 0) {
    errors.triggerPhrases = "At least one trigger phrase is required.";
  }

  if (model.actions.length === 0) {
    errors.actions = "At least one action is required.";
  }

  model.actions.forEach((action, index) => {
    if (isEditorPendingAction(action)) {
      errors.actionErrors[index] = "Choose an action type.";
      return;
    }
    if ("open_url" in action) {
      const maybeError = validateUrl(action.open_url.url);
      if (maybeError) {
        errors.actionErrors[index] = maybeError;
      }
    }
    if ("http_get" in action) {
      const maybeError = validateUrl(action.http_get.url);
      if (maybeError) {
        errors.actionErrors[index] = maybeError;
      }
    }
    if ("read_file" in action) {
      if (action.read_file.path.trim().length === 0) {
        errors.actionErrors[index] = "File path is required.";
      }
    }
    if ("list_folder" in action) {
      if (action.list_folder.path.trim().length === 0) {
        errors.actionErrors[index] = "Folder path is required.";
      }
    }
    if ("write_file" in action) {
      if (action.write_file.path.trim().length === 0) {
        errors.actionErrors[index] = "File path is required.";
      }
    }
    if ("get_file_metadata" in action) {
      if (action.get_file_metadata.path.trim().length === 0) {
        errors.actionErrors[index] = "File path is required.";
      }
    }
    if ("show_notification" in action) {
      const title = action.show_notification.title.trim();
      const body = action.show_notification.body.trim();
      if (title.length === 0 && body.length === 0) {
        errors.actionErrors[index] = "Notification needs a title or body.";
      }
    }
    if ("text_match" in action) {
      if (action.text_match.pattern.trim().length === 0) {
        errors.actionErrors[index] = "Regex pattern is required.";
      }
    }
    if ("text_split" in action) {
      if (action.text_split.delimiter.length === 0) {
        errors.actionErrors[index] = "Delimiter is required.";
      }
    }
    if ("if_else" in action) {
      const branchErrors = validateIfElseFields(action.if_else);
      const msg =
        branchErrors.pattern ?? branchErrors.then ?? branchErrors.else ?? branchErrors.text;
      if (msg) {
        errors.actionErrors[index] = msg;
      }
    }
    if ("sub_prompt" in action) {
      if (action.sub_prompt.prompt.trim().length === 0) {
        errors.actionErrors[index] = "Follow-up text is required.";
      }
    }
  });

  return errors;
}

export function hasBlockingErrors(errors: FormErrors): boolean {
  return Boolean(
    errors.triggerPhrases || errors.actions || Object.keys(errors.actionErrors).length > 0,
  );
}

export function parseTriggerPhraseInput(value: string): string[] {
  return value
    .split(",")
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
}

function normalizeTriggerPhrases(phrases: string[]): string[] {
  return phrases.map((phrase) => phrase.trim()).filter((phrase) => phrase.length > 0);
}

function validateUrl(value: string): string | null {
  if (value.trim().length === 0) {
    return "URL is required.";
  }
  try {
    const parsed = new URL(value);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return "URL must use http or https.";
    }
    return null;
  } catch {
    return "URL is invalid.";
  }
}

import type { ActionPayload, CommandNodePayload } from "../../types";

export type ActionKind =
  | "open_app"
  | "open_url"
  | "run_script"
  | "send_keys"
  | "speak"
  | "wait"
  | "sub_prompt";

export type FormModel = {
  id: number | null;
  triggerPhrases: string[];
  enabled: boolean;
  actions: ActionPayload[];
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

export function defaultActionForKind(kind: ActionKind): ActionPayload {
  switch (kind) {
    case "open_app":
      return { open_app: { name: "", path: "" } };
    case "open_url":
      return { open_url: { url: "" } };
    case "run_script":
      return { run_script: { script: "", args: [] } };
    case "send_keys":
      return { send_keys: { keys: "" } };
    case "speak":
      return { speak: { text: "" } };
    case "wait":
      return { wait: { ms: 250 } };
    case "sub_prompt":
      return { sub_prompt: { prompt: "" } };
  }
}

export function emptyFormModel(): FormModel {
  return {
    id: null,
    triggerPhrases: [],
    enabled: true,
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
    actions: [...model.actions],
    enabled: model.enabled,
    fuzzy_threshold_pct: 0,
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
    if ("open_url" in action) {
      const maybeError = validateUrl(action.open_url.url);
      if (maybeError) {
        errors.actionErrors[index] = maybeError;
      }
    }
    if ("sub_prompt" in action) {
      if (action.sub_prompt.prompt.trim().length === 0) {
        errors.actionErrors[index] = "Sub-prompt text is required.";
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

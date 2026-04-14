import type { ActionPayload, CommandNodePayload } from "../../types";

export type ActionKind = "open_app" | "open_url" | "run_script" | "send_keys" | "speak" | "wait";

export type FormModel = {
  id: number | null;
  name: string;
  triggerPhrases: string[];
  threshold: number;
  enabled: boolean;
  actions: ActionPayload[];
  subPromptText: string;
  subPromptActions: ActionPayload[];
};

export type FormErrors = {
  name?: string;
  triggerPhrases?: string;
  threshold?: string;
  actions?: string;
  actionUrls: Record<number, string>;
  subPromptText?: string;
  subPromptUrls: Record<number, string>;
};

const MIN_THRESHOLD = 0.5;
const MAX_THRESHOLD = 1;

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
  }
}

export function emptyFormModel(): FormModel {
  return {
    id: null,
    name: "",
    triggerPhrases: [],
    threshold: 0.8,
    enabled: true,
    actions: [],
    subPromptText: "",
    subPromptActions: [],
  };
}

export function modelFromNode(node: CommandNodePayload | null): FormModel {
  if (!node) {
    return emptyFormModel();
  }
  const normalizedSubPrompt = node.sub_prompt?.trim() ?? "";
  const subPromptIndex = node.actions.findIndex((action) => "sub_prompt" in action);
  if (subPromptIndex === -1) {
    return {
      id: node.id,
      name: node.name,
      triggerPhrases: [...node.trigger_phrases],
      threshold: clampThreshold(node.fuzzy_threshold_pct / 100),
      enabled: node.enabled,
      actions: [...node.actions],
      subPromptText: normalizedSubPrompt,
      subPromptActions: [],
    };
  }

  const subPromptAction = node.actions[subPromptIndex];
  return {
    id: node.id,
    name: node.name,
    triggerPhrases: [...node.trigger_phrases],
    threshold: clampThreshold(node.fuzzy_threshold_pct / 100),
    enabled: node.enabled,
    actions: node.actions.slice(0, subPromptIndex),
    subPromptText: "sub_prompt" in subPromptAction ? subPromptAction.sub_prompt.prompt : "",
    subPromptActions: node.actions.slice(subPromptIndex + 1),
  };
}

export function toCommandPayload(model: FormModel): Omit<CommandNodePayload, "id" | "created_at"> {
  const mergedActions = [...model.actions];
  const normalizedSubPrompt = model.subPromptText.trim();
  if (model.subPromptText.trim().length > 0 || model.subPromptActions.length > 0) {
    mergedActions.push({ sub_prompt: { prompt: normalizedSubPrompt } });
    mergedActions.push(...model.subPromptActions);
  }
  return {
    name: model.name.trim(),
    trigger_phrases: normalizeTriggerPhrases(model.triggerPhrases),
    actions: mergedActions,
    enabled: model.enabled,
    fuzzy_threshold_pct: Math.round(clampThreshold(model.threshold) * 100),
    ai_mode: normalizedSubPrompt.length > 0,
    sub_prompt: normalizedSubPrompt.length > 0 ? normalizedSubPrompt : null,
  };
}

export function validateFormModel(model: FormModel): FormErrors {
  const errors: FormErrors = {
    actionUrls: {},
    subPromptUrls: {},
  };

  if (model.name.trim().length === 0) {
    errors.name = "Name is required.";
  }

  if (normalizeTriggerPhrases(model.triggerPhrases).length === 0) {
    errors.triggerPhrases = "At least one trigger phrase is required.";
  }

  if (model.threshold < MIN_THRESHOLD || model.threshold > MAX_THRESHOLD) {
    errors.threshold = "Threshold must be between 0.50 and 1.00.";
  }

  const topLevelCount = model.actions.length;
  const subPromptCount = model.subPromptActions.length;
  if (topLevelCount + subPromptCount === 0) {
    errors.actions = "At least one action is required.";
  }

  model.actions.forEach((action, index) => {
    if ("open_url" in action) {
      const maybeError = validateUrl(action.open_url.url);
      if (maybeError) {
        errors.actionUrls[index] = maybeError;
      }
    }
  });

  if (model.subPromptActions.length > 0 && model.subPromptText.trim().length === 0) {
    errors.subPromptText = "Sub-prompt text is required when sub-prompt actions exist.";
  }

  model.subPromptActions.forEach((action, index) => {
    if ("open_url" in action) {
      const maybeError = validateUrl(action.open_url.url);
      if (maybeError) {
        errors.subPromptUrls[index] = maybeError;
      }
    }
  });

  return errors;
}

export function hasBlockingErrors(errors: FormErrors): boolean {
  return Boolean(
    errors.name ||
      errors.triggerPhrases ||
      errors.threshold ||
      errors.actions ||
      errors.subPromptText ||
      Object.keys(errors.actionUrls).length > 0 ||
      Object.keys(errors.subPromptUrls).length > 0,
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

function clampThreshold(value: number): number {
  return Math.min(MAX_THRESHOLD, Math.max(MIN_THRESHOLD, value));
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

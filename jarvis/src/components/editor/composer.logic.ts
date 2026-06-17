import type {
  GenerateAutomationResult,
  NewToolDefinitionPayload,
} from "../../types";
import type { FormModel } from "./NodeForm.logic";

/** Map IPC `generate_automation` output into the draft formula editor model. */
export function modelFromGeneratedResult(result: GenerateAutomationResult): FormModel | null {
  if ((result.kind !== "command" && result.kind !== "both") || !result.command) {
    return null;
  }
  return {
    id: null,
    triggerPhrases: [...result.command.trigger_phrases],
    enabled: result.command.enabled,
    prefixMode: result.command.match_mode === "prefix",
    actions: [...result.command.actions],
  };
}

export function toolDraftFromResult(
  result: GenerateAutomationResult,
): NewToolDefinitionPayload | null {
  if (!result.tool) {
    return null;
  }
  return {
    name: result.tool.name,
    display_name: result.tool.display_name,
    description: result.tool.description,
    parameters: result.tool.parameters.map((param) => ({ ...param })),
    actions: [...result.tool.actions],
    enabled: result.tool.enabled,
    builtin: false,
  };
}

/** Payload shape expected by `create_tool` IPC. */
export function toCreateToolPayload(
  tool: NewToolDefinitionPayload,
): NewToolDefinitionPayload {
  return {
    name: tool.name,
    display_name: tool.display_name,
    description: tool.description,
    parameters: tool.parameters.map((param) => ({ ...param })),
    actions: [...tool.actions],
    enabled: tool.enabled,
    builtin: false,
  };
}

export function shouldShowToolPreview(result: GenerateAutomationResult): boolean {
  return (result.kind === "tool" || result.kind === "both") && Boolean(result.tool);
}

export function shouldShowFormulaAfterGenerate(result: GenerateAutomationResult): boolean {
  return (result.kind === "command" || result.kind === "both") && Boolean(result.command);
}

/** True when the composer Generate button should be enabled. */
export function canComposerGenerate(trigger: string, description: string): boolean {
  return description.trim().length > 0;
}

/** Normalize optional trigger for IPC — empty string becomes null. */
export function composerTriggerPayload(trigger: string): string | null {
  const trimmed = trigger.trim();
  return trimmed.length > 0 ? trimmed : null;
}

const COMPOSER_VALIDATION_ERROR_CODES = new Set(["schema_invalid", "invalid_json"]);

export type ParsedComposerError = {
  code: string | null;
  message: string;
};

/** Parse `generate_automation` JSON error payload from Tauri invoke failures. */
export function parseComposerInvokeError(err: unknown): ParsedComposerError {
  const raw = err instanceof Error ? err.message : String(err);
  const trimmed = raw.trim();
  if (!trimmed) {
    return { code: null, message: raw };
  }
  if (trimmed.startsWith("{")) {
    try {
      const parsed = JSON.parse(trimmed) as { code?: string; message?: string };
      if (typeof parsed.message === "string" && parsed.message.length > 0) {
        return {
          code: typeof parsed.code === "string" ? parsed.code : null,
          message: parsed.message,
        };
      }
    } catch {
      /* fall through */
    }
  }
  return { code: null, message: trimmed };
}

/** True when backend validation/repair failed and another Generate may help. */
export function shouldOfferRegenerateWithFix(err: unknown): boolean {
  const { code } = parseComposerInvokeError(err);
  return code !== null && COMPOSER_VALIDATION_ERROR_CODES.has(code);
}

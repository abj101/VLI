import { describe, expect, it } from "vitest";
import {
  modelFromGeneratedResult,
  parseComposerInvokeError,
  canComposerGenerate,
  composerPlaceholdersForPlatform,
  composerTriggerPayload,
  shouldOfferRegenerateWithFix,
  shouldShowFormulaAfterGenerate,
  shouldShowToolPreview,
  toCreateToolPayload,
  toolDraftFromResult,
} from "./composer.logic";
import type { GenerateAutomationResult } from "../../types";

const openNotepadResult: GenerateAutomationResult = {
  kind: "command",
  confidence: 0.9,
  summary: "Open Notepad on the left",
  warnings: [],
  command: {
    name: "open notepad",
    trigger_phrases: ["open notepad"],
    actions: [{ open_target: { target: "notepad", placement: "left_half" } }],
    enabled: true,
    fuzzy_threshold_pct: 0,
    match_mode: "phrase",
  },
};

const speakToolResult: GenerateAutomationResult = {
  kind: "tool",
  confidence: 0.85,
  summary: "Speak arbitrary text",
  warnings: [],
  tool: {
    name: "speak_text",
    display_name: "Speak text",
    description: "Speak what the user says",
    parameters: [{ name: "text", param_type: "string", required: true }],
    actions: [{ speak: { text: "{{text}}" } }],
    enabled: true,
  },
};

const bothResult: GenerateAutomationResult = {
  kind: "both",
  confidence: 0.88,
  summary: "Speak with trigger",
  warnings: [],
  command: {
    name: "speak",
    trigger_phrases: ["speak"],
    match_mode: "prefix",
    actions: [{ speak: { text: "{{text}}" } }],
    enabled: true,
    fuzzy_threshold_pct: 0,
  },
  tool: {
    name: "speak_custom",
    display_name: "Speak",
    description: "Speak text",
    parameters: [{ name: "text", param_type: "string", required: true }],
    actions: [{ speak: { text: "{{text}}" } }],
    enabled: true,
  },
};

describe("composer.logic", () => {
  it("maps command result to FormModel", () => {
    const model = modelFromGeneratedResult(openNotepadResult);
    expect(model).not.toBeNull();
    expect(model?.triggerPhrases).toEqual(["open notepad"]);
    expect(model?.prefixMode).toBe(false);
    expect(model?.actions).toEqual([
      { open_target: { target: "notepad", placement: "left_half" } },
    ]);
  });

  it("maps both result to FormModel", () => {
    const model = modelFromGeneratedResult(bothResult);
    expect(model?.triggerPhrases).toEqual(["speak"]);
    expect(model?.prefixMode).toBe(true);
  });

  it("returns null for tool-only results", () => {
    expect(modelFromGeneratedResult(speakToolResult)).toBeNull();
  });

  it("extracts tool draft from result", () => {
    const draft = toolDraftFromResult(speakToolResult);
    expect(draft?.name).toBe("speak_text");
    expect(draft?.parameters).toHaveLength(1);
  });

  it("toCreateToolPayload forces builtin false", () => {
    const draft = toolDraftFromResult(speakToolResult)!;
    const payload = toCreateToolPayload(draft);
    expect(payload.builtin).toBe(false);
    expect(payload.name).toBe("speak_text");
  });

  it("shows tool preview for tool and both", () => {
    expect(shouldShowToolPreview(speakToolResult)).toBe(true);
    expect(shouldShowToolPreview(bothResult)).toBe(true);
    expect(shouldShowToolPreview(openNotepadResult)).toBe(false);
  });

  it("shows formula for command and both", () => {
    expect(shouldShowFormulaAfterGenerate(openNotepadResult)).toBe(true);
    expect(shouldShowFormulaAfterGenerate(bothResult)).toBe(true);
    expect(shouldShowFormulaAfterGenerate(speakToolResult)).toBe(false);
  });

  it("parses composer JSON invoke errors", () => {
    const err = new Error(
      JSON.stringify({ code: "schema_invalid", message: "trigger_phrases cannot be empty" }),
    );
    expect(parseComposerInvokeError(err)).toEqual({
      code: "schema_invalid",
      message: "trigger_phrases cannot be empty",
    });
    expect(shouldOfferRegenerateWithFix(err)).toBe(true);
  });

  it("does not offer regenerate for non-validation errors", () => {
    const err = new Error(JSON.stringify({ code: "timed_out", message: "Composer inference timed out" }));
    expect(shouldOfferRegenerateWithFix(err)).toBe(false);
  });

  it("requires description before generate", () => {
    expect(canComposerGenerate("", "")).toBe(false);
    expect(canComposerGenerate("open notepad", "")).toBe(false);
    expect(canComposerGenerate("", "launch Notepad")).toBe(true);
    expect(canComposerGenerate("open notepad", "launch Notepad")).toBe(true);
  });

  it("normalizes trigger payload for IPC", () => {
    expect(composerTriggerPayload("  open notepad  ")).toBe("open notepad");
    expect(composerTriggerPayload("   ")).toBeNull();
    expect(composerTriggerPayload("")).toBeNull();
  });

  it("returns macOS composer placeholders", () => {
    expect(composerPlaceholdersForPlatform("macos")).toEqual({
      trigger: "e.g. open TextEdit",
      description: "e.g. open TextEdit, create a new note, and paste hello world",
    });
  });

  it("returns Windows composer placeholders by default", () => {
    expect(composerPlaceholdersForPlatform("windows")).toEqual({
      trigger: "e.g. open notepad",
      description: "e.g. launch Notepad snapped left",
    });
    expect(composerPlaceholdersForPlatform(undefined)).toEqual({
      trigger: "e.g. open notepad",
      description: "e.g. launch Notepad snapped left",
    });
  });
});

import { describe, expect, it } from "vitest";
import type { CommandNodePayload } from "../../types";
import { editorPendingAction } from "../../types";
import {
  defaultActionForKind,
  derivedCommandName,
  hasBlockingErrors,
  modelFromNode,
  toCommandPayload,
  validateFormModel,
} from "./NodeForm.logic";

function makeNode(): CommandNodePayload {
  return {
    id: 21,
    name: "Search docs",
    trigger_phrases: ["search docs", "docs search"],
    actions: [
      { open_url: { url: "https://example.com" } },
      { sub_prompt: { prompt: "What should I search?" } },
      { open_url: { url: "https://example.com/search?q={{follow_up}}" } },
    ],
    enabled: true,
    fuzzy_threshold_pct: 82,
    created_at: "2026-01-01T00:00:00Z",
  };
}

describe("NodeForm logic", () => {
  it("loads trigger phrases and actions from the node", () => {
    const model = modelFromNode(makeNode());
    expect(model.triggerPhrases).toEqual(["search docs", "docs search"]);
    expect(model.actions).toEqual(makeNode().actions);
  });

  it("derives stored name from first trigger and uses global fuzzy (0)", () => {
    const model = modelFromNode(makeNode());
    const payload = toCommandPayload(model);
    expect(payload.name).toBe(derivedCommandName(model.triggerPhrases));
    expect(payload.name).toBe("search docs");
    expect(payload.trigger_phrases).toEqual(["search docs", "docs search"]);
    expect(payload.fuzzy_threshold_pct).toBe(0);
  });

  it("validates required fields and blocks save", () => {
    const errors = validateFormModel({
      id: null,
      triggerPhrases: [],
      enabled: true,
      actions: [],
    });

    expect(errors.triggerPhrases).toBeTruthy();
    expect(errors.actions).toBeTruthy();
    expect(hasBlockingErrors(errors)).toBe(true);
  });

  it("validates open_url actions by index", () => {
    const errors = validateFormModel({
      id: null,
      triggerPhrases: ["open thing"],
      enabled: true,
      actions: [
        { open_url: { url: "not-a-url" } },
        { sub_prompt: { prompt: "Where?" } },
        { open_url: { url: "" } },
      ],
    });

    expect(errors.actionErrors[0]).toBe("URL is invalid.");
    expect(errors.actionErrors[2]).toBe("URL is required.");
    expect(hasBlockingErrors(errors)).toBe(true);
  });

  it("requires non-empty follow-up text on sub_prompt actions", () => {
    const errors = validateFormModel({
      id: null,
      triggerPhrases: ["ask me"],
      enabled: true,
      actions: [{ wait: { ms: 100 } }, { sub_prompt: { prompt: "   " } }, { open_url: { url: "https://example.com" } }],
    });
    expect(errors.actionErrors[1]).toBe("Follow-up text is required.");
    expect(hasBlockingErrors(errors)).toBe(true);
  });

  it("flags editor-pending rows until a type is chosen", () => {
    const errors = validateFormModel({
      id: null,
      triggerPhrases: ["go"],
      enabled: true,
      actions: [editorPendingAction()],
    });
    expect(errors.actionErrors[0]).toBe("Choose an action type.");
    expect(hasBlockingErrors(errors)).toBe(true);
  });

  it("strips editor-pending slots from command payload", () => {
    const payload = toCommandPayload({
      id: 2,
      triggerPhrases: ["hi"],
      enabled: true,
      actions: [editorPendingAction(), { open_url: { url: "https://example.com" } }],
    });
    expect(payload.actions).toEqual([{ open_url: { url: "https://example.com" } }]);
  });

  it("maps prefix mode to match_mode prefix in payload", () => {
    const payload = toCommandPayload({
      id: 3,
      triggerPhrases: ["open"],
      enabled: true,
      prefixMode: true,
      actions: [{ open_app: { name: "{{remainder}}", path: "" } }],
    });
    expect(payload.match_mode).toBe("prefix");
  });

  it("defaultActionForKind returns expected shape", () => {
    expect(defaultActionForKind("open_app")).toEqual({ open_app: { name: "", path: "" } });
    expect(defaultActionForKind("open_url")).toEqual({ open_url: { url: "" } });
    expect(defaultActionForKind("run_script")).toEqual({ run_script: { script: "", args: [] } });
    expect(defaultActionForKind("send_keys")).toEqual({ send_keys: { keys: "" } });
    expect(defaultActionForKind("speak")).toEqual({ speak: { text: "" } });
    expect(defaultActionForKind("wait")).toEqual({ wait: { ms: 0 } });
    expect(defaultActionForKind("sub_prompt")).toEqual({ sub_prompt: { prompt: "" } });
    expect(defaultActionForKind("text_match")).toEqual({
      text_match: { pattern: "", text: "" },
    });
    expect(defaultActionForKind("text_combine")).toEqual({ text_combine: { separator: "" } });
    expect(defaultActionForKind("list_folder")).toEqual({ list_folder: { path: "" } });
    expect(defaultActionForKind("write_file")).toEqual({
      write_file: { path: "", content: "" },
    });
    expect(defaultActionForKind("device_info")).toEqual({ device_info: {} });
  });

  it("requires folder path on list_folder actions", () => {
    const errors = validateFormModel({
      id: null,
      triggerPhrases: ["browse"],
      enabled: true,
      actions: [{ list_folder: { path: "  " } }],
    });
    expect(errors.actionErrors[0]).toBe("Folder path is required.");
    expect(hasBlockingErrors(errors)).toBe(true);
  });

  it("requires file path on write_file actions", () => {
    const errors = validateFormModel({
      id: null,
      triggerPhrases: ["save"],
      enabled: true,
      actions: [{ write_file: { path: "", content: "{{last_result}}" } }],
    });
    expect(errors.actionErrors[0]).toBe("File path is required.");
    expect(hasBlockingErrors(errors)).toBe(true);
  });

  it("requires regex pattern on text_match actions", () => {
    const errors = validateFormModel({
      id: null,
      triggerPhrases: ["parse"],
      enabled: true,
      actions: [{ text_match: { pattern: "  ", text: "" } }],
    });
    expect(errors.actionErrors[0]).toBe("Regex pattern is required.");
    expect(hasBlockingErrors(errors)).toBe(true);
  });

  it("requires delimiter on text_split actions", () => {
    const errors = validateFormModel({
      id: null,
      triggerPhrases: ["split"],
      enabled: true,
      actions: [{ text_split: { delimiter: "", text: "{{last_result}}" } }],
    });
    expect(errors.actionErrors[0]).toBe("Delimiter is required.");
    expect(hasBlockingErrors(errors)).toBe(true);
  });
});

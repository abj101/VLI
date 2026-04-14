import { describe, expect, it } from "vitest";
import type { CommandNodePayload } from "../../types";
import {
  defaultActionForKind,
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
  it("splits node actions into main and sub-prompt sections", () => {
    const model = modelFromNode(makeNode());
    expect(model.actions).toEqual([{ open_url: { url: "https://example.com" } }]);
    expect(model.subPromptText).toBe("What should I search?");
    expect(model.subPromptActions).toEqual([
      { open_url: { url: "https://example.com/search?q={{follow_up}}" } },
    ]);
  });

  it("joins model data into command payload with sub-prompt chain", () => {
    const model = modelFromNode(makeNode());
    const payload = toCommandPayload(model);
    expect(payload.name).toBe("Search docs");
    expect(payload.trigger_phrases).toEqual(["search docs", "docs search"]);
    expect(payload.fuzzy_threshold_pct).toBe(82);
    expect(payload.actions).toEqual(makeNode().actions);
  });

  it("validates required fields and blocks save", () => {
    const errors = validateFormModel({
      id: null,
      name: "  ",
      triggerPhrases: [],
      threshold: 0.45,
      enabled: true,
      actions: [],
      subPromptText: "",
      subPromptActions: [],
    });

    expect(errors.name).toBeTruthy();
    expect(errors.triggerPhrases).toBeTruthy();
    expect(errors.threshold).toBeTruthy();
    expect(errors.actions).toBeTruthy();
    expect(hasBlockingErrors(errors)).toBe(true);
  });

  it("validates open_url actions in both chains", () => {
    const errors = validateFormModel({
      id: null,
      name: "Open stuff",
      triggerPhrases: ["open thing"],
      threshold: 0.8,
      enabled: true,
      actions: [{ open_url: { url: "not-a-url" } }],
      subPromptText: "Where?",
      subPromptActions: [{ open_url: { url: "" } }],
    });

    expect(errors.actionUrls[0]).toBe("URL is invalid.");
    expect(errors.subPromptUrls[0]).toBe("URL is required.");
    expect(hasBlockingErrors(errors)).toBe(true);
  });

  it("requires sub-prompt text when nested actions exist", () => {
    const errors = validateFormModel({
      id: null,
      name: "Need follow-up",
      triggerPhrases: ["ask me"],
      threshold: 0.75,
      enabled: true,
      actions: [{ wait: { ms: 100 } }],
      subPromptText: "",
      subPromptActions: [{ open_url: { url: "https://example.com" } }],
    });
    expect(errors.subPromptText).toBeTruthy();
    expect(hasBlockingErrors(errors)).toBe(true);
  });

  it("defaultActionForKind returns expected shape", () => {
    expect(defaultActionForKind("open_app")).toEqual({ open_app: { name: "", path: "" } });
    expect(defaultActionForKind("open_url")).toEqual({ open_url: { url: "" } });
    expect(defaultActionForKind("run_script")).toEqual({ run_script: { script: "", args: [] } });
    expect(defaultActionForKind("send_keys")).toEqual({ send_keys: { keys: "" } });
    expect(defaultActionForKind("speak")).toEqual({ speak: { text: "" } });
    expect(defaultActionForKind("wait")).toEqual({ wait: { ms: 250 } });
  });
});

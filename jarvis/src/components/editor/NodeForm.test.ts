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
  it("loads all actions from the node in order", () => {
    const model = modelFromNode(makeNode());
    expect(model.actions).toEqual(makeNode().actions);
  });

  it("round-trips command payload actions unchanged", () => {
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
    });

    expect(errors.name).toBeTruthy();
    expect(errors.triggerPhrases).toBeTruthy();
    expect(errors.threshold).toBeTruthy();
    expect(errors.actions).toBeTruthy();
    expect(hasBlockingErrors(errors)).toBe(true);
  });

  it("validates open_url actions by index", () => {
    const errors = validateFormModel({
      id: null,
      name: "Open stuff",
      triggerPhrases: ["open thing"],
      threshold: 0.8,
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

  it("requires non-empty sub-prompt text on sub_prompt actions", () => {
    const errors = validateFormModel({
      id: null,
      name: "Need follow-up",
      triggerPhrases: ["ask me"],
      threshold: 0.75,
      enabled: true,
      actions: [{ wait: { ms: 100 } }, { sub_prompt: { prompt: "   " } }, { open_url: { url: "https://example.com" } }],
    });
    expect(errors.actionErrors[1]).toBe("Sub-prompt text is required.");
    expect(hasBlockingErrors(errors)).toBe(true);
  });

  it("defaultActionForKind returns expected shape", () => {
    expect(defaultActionForKind("open_app")).toEqual({ open_app: { name: "", path: "" } });
    expect(defaultActionForKind("open_url")).toEqual({ open_url: { url: "" } });
    expect(defaultActionForKind("run_script")).toEqual({ run_script: { script: "", args: [] } });
    expect(defaultActionForKind("send_keys")).toEqual({ send_keys: { keys: "" } });
    expect(defaultActionForKind("speak")).toEqual({ speak: { text: "" } });
    expect(defaultActionForKind("wait")).toEqual({ wait: { ms: 250 } });
    expect(defaultActionForKind("sub_prompt")).toEqual({ sub_prompt: { prompt: "" } });
  });
});

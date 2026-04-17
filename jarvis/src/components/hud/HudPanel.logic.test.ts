import { describe, expect, it } from "vitest";
import type { HudPhase } from "../../types";
import {
  announcableText,
  selectCenterContent,
  selectPhaseLabel,
} from "./HudPanel.logic";

function base(phase: HudPhase) {
  return {
    phase,
    transcript: "",
    match: null,
    actionText: null,
    actionError: null,
    audioError: null,
  };
}

describe("selectCenterContent", () => {
  it("shows transcript while listening with no match", () => {
    const out = selectCenterContent({
      ...base("listening"),
      transcript: "hello there",
    });
    expect(out).toEqual({ kind: "transcript", text: "hello there" });
  });

  it("shows action status in executing when available", () => {
    const out = selectCenterContent({
      ...base("executing"),
      actionText: "Opening Notepad...",
    });
    expect(out).toEqual({ kind: "action", text: "Opening Notepad..." });
  });

  it("shows working fallback during matched or executing before match/action hydrate", () => {
    expect(selectCenterContent(base("matched"))).toEqual({
      kind: "action",
      text: "Working…",
    });
    expect(selectCenterContent(base("executing"))).toEqual({
      kind: "action",
      text: "Working…",
    });
  });

  it("shows action error when executor reports a terminal failure", () => {
    const out = selectCenterContent({
      ...base("executing"),
      actionError: "Failed to run chain",
      actionText: "Opening Notepad...",
    });
    expect(out).toEqual({ kind: "error", text: "Failed to run chain" });
  });

  it("shows audio error while awaiting follow-up input", () => {
    const out = selectCenterContent({
      ...base("awaiting_input"),
      audioError: "Microphone unavailable",
    });
    expect(out).toEqual({ kind: "error", text: "Microphone unavailable" });
  });

  it("shows live transcript during awaiting_input when status is not follow-up prompt", () => {
    const out = selectCenterContent({
      ...base("awaiting_input"),
      transcript: "rust tauri",
      actionText: "Searching docs...",
    });
    expect(out).toEqual({ kind: "transcript", text: "rust tauri" });
  });

  it("normalizes follow-up prompt action text", () => {
    const out = selectCenterContent({
      ...base("awaiting_input"),
      actionText: "Awaiting input: What should I search on GitHub?",
    });
    expect(out).toEqual({ kind: "action", text: "follow up" });
  });
});

describe("announcableText", () => {
  it("returns matched phrase for match kind", () => {
    const input = {
      ...base("matched"),
      match: {
        node_id: "n1",
        matched_phrase: "open notepad",
        span_start: 0,
        span_end: 12,
      },
    };
    const selected = selectCenterContent(input);
    expect(selected.kind).toBe("match");
    expect(announcableText(input, selected)).toBe("open notepad");
  });

  it("returns empty string for placeholder", () => {
    const input = base("listening");
    const selected = selectCenterContent(input);
    expect(selected.kind).toBe("placeholder");
    expect(announcableText(input, selected)).toBe("");
  });
});

describe("selectPhaseLabel", () => {
  it("does not show a phase line label (state is conveyed by content + motion)", () => {
    expect(selectPhaseLabel("listening")).toBeNull();
    expect(selectPhaseLabel("matched")).toBeNull();
    expect(selectPhaseLabel("awaiting_input")).toBeNull();
    expect(selectPhaseLabel("executing")).toBeNull();
    expect(selectPhaseLabel("done")).toBeNull();
    expect(selectPhaseLabel("stopped")).toBeNull();
    expect(selectPhaseLabel("idle")).toBeNull();
  });
});

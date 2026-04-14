import { describe, expect, it } from "vitest";
import type { HudPhase } from "../../types";
import { selectCenterContent, selectPhaseLabel } from "./HudPanel.logic";

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

describe("selectPhaseLabel", () => {
  it("hides all status labels in HUD corner", () => {
    expect(selectPhaseLabel("listening")).toBeNull();
    expect(selectPhaseLabel("matched")).toBeNull();
    expect(selectPhaseLabel("awaiting_input")).toBeNull();
    expect(selectPhaseLabel("executing")).toBeNull();
    expect(selectPhaseLabel("done")).toBeNull();
    expect(selectPhaseLabel("stopped")).toBeNull();
  });
});

import { describe, expect, it } from "vitest";
import type { HudPhase } from "../../types";
import { selectCenterContent, selectPhaseLabel } from "./HudPanel";

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

  it("shows live transcript during awaiting_input even when prompt text exists", () => {
    const out = selectCenterContent({
      ...base("awaiting_input"),
      transcript: "rust tauri",
      actionText: "Awaiting input: What should I search on GitHub?",
    });
    expect(out).toEqual({ kind: "transcript", text: "rust tauri" });
  });
});

describe("selectPhaseLabel", () => {
  it("hides listening and awaiting labels, keeps execution labels", () => {
    expect(selectPhaseLabel("listening")).toBeNull();
    expect(selectPhaseLabel("matched")).toBe("Matched");
    expect(selectPhaseLabel("awaiting_input")).toBeNull();
    expect(selectPhaseLabel("executing")).toBe("Executing");
    expect(selectPhaseLabel("done")).toBe("Done");
    expect(selectPhaseLabel("stopped")).toBe("Stopped");
  });
});

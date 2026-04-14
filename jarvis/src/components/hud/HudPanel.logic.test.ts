import { describe, expect, it } from "vitest";
import type { HudPhase } from "../../types";
import { selectCenterContent } from "./HudPanel";

function base(phase: HudPhase) {
  return {
    phase,
    transcript: "",
    match: null,
    actionText: null,
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
});

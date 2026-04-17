import { describe, expect, it } from "vitest";
import {
  HUD_OVERLAY_SHELL_PHASES,
  isHudOverlayShellActive,
} from "./hudOverlayPhases";

describe("isHudOverlayShellActive", () => {
  it("is true only for in-session overlay phases", () => {
    for (const p of HUD_OVERLAY_SHELL_PHASES) {
      expect(isHudOverlayShellActive(p)).toBe(true);
    }
    expect(isHudOverlayShellActive("done")).toBe(false);
    expect(isHudOverlayShellActive("idle")).toBe(false);
    expect(isHudOverlayShellActive("stopped")).toBe(false);
  });
});

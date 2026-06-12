import { describe, expect, it } from "vitest";
import type { OpenTargetPreview } from "../../types";
import { formatOpenTargetPreview, formatResolvedTargetSummary } from "./toolsTab.logic";

describe("toolsTab.logic", () => {
  it("formats open target preview lines", () => {
    const preview: OpenTargetPreview = {
      utterance: "open brave on the left",
      target: "brave",
      placement: "left_half",
      resolved: {
        app: { display_name: "Brave", exe_path: "C:\\Brave\\brave.exe" },
      },
    };
    const lines = formatOpenTargetPreview(preview);
    expect(lines.targetLine).toBe("brave");
    expect(lines.placementLine).toBe("Left half");
    expect(lines.resolvedLine).toContain("Brave");
  });

  it("summarizes ambiguous resolution", () => {
    const summary = formatResolvedTargetSummary({
      ambiguous: {
        query: "github",
        app_candidates: [{ display_name: "GitHub", exe_path: "gh.exe", score: 0.8 }],
        url_candidate: { url: "https://github.com", score: 0.75 },
      },
    });
    expect(summary).toContain("Ambiguous");
    expect(summary).toContain("GitHub");
  });
});

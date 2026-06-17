import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { CommandNodePayload } from "../../types";
import { CommandFormulaRow } from "./CommandFormulaRow";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function ifElseNode(): CommandNodePayload {
  return {
    id: 501,
    name: "branch demo",
    trigger_phrases: ["branch demo"],
    actions: [
      {
        if_else: {
          condition: "text_contains",
          text: "{{last_result}}",
          pattern: "go",
          then_actions: [{ speak: { text: "yes" } }],
          else_actions: [{ speak: { text: "no" } }],
        },
      },
    ],
    enabled: true,
    fuzzy_threshold_pct: 0,
    created_at: "2026-04-19T00:00:00Z",
  };
}

describe("CommandFormulaRow If/Else layout", () => {
  it("renders indented Then/Else branch sub-rows with bracket labels", () => {
    const html = renderToStaticMarkup(
      <CommandFormulaRow node={ifElseNode()} onToggleEnabled={() => {}} onDelete={() => {}} />,
    );
    expect(html).toContain("editor-formula-ifelse-branches");
    expect(html).toContain("editor-formula-branch");
    expect(html).toContain(">Then<");
    expect(html).toContain(">Else<");
    expect(html).toContain("Text contains");
  });
});

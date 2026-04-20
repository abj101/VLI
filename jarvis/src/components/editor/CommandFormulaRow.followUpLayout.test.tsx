import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { CommandNodePayload } from "../../types";
import { CommandFormulaRow } from "./CommandFormulaRow";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function followUpNode(): CommandNodePayload {
  return {
    id: 401,
    name: "demo",
    trigger_phrases: ["demo"],
    actions: [{ sub_prompt: { prompt: "Hello?" } }, { open_url: { url: "https://example.com/path" } }],
    enabled: true,
    fuzzy_threshold_pct: 0,
    created_at: "2026-04-19T00:00:00Z",
  };
}

describe("CommandFormulaRow follow-up layout", () => {
  it("renders a full-width variable bracket under the kind + follow-up inputs", () => {
    const html = renderToStaticMarkup(
      <CommandFormulaRow node={followUpNode()} onToggleEnabled={() => {}} onDelete={() => {}} />,
    );
    expect(html).toContain("editor-formula-variable-anchor");
    expect(html).toContain("editor-formula-variable-inputs-row");
    expect(html).toContain("editor-formula-variable-bracket-svg");
    expect(html).toContain(">Variable 1<");
  });

  it("places the inline remove control inside the clearable arg slot", () => {
    const html = renderToStaticMarkup(
      <CommandFormulaRow node={followUpNode()} onToggleEnabled={() => {}} onDelete={() => {}} />,
    );
    expect(html).toMatch(
      /class="editor-formula-arg-slot editor-formula-arg-slot--clearable"[^>]*>[\s\S]*?editor-formula-remove-inline/,
    );
  });
});

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

/** DOM shape for a new pending step: remove lives inside the kind ("Action") wrap, not a separate arg column. */
function PendingKindRemoveHarness() {
  return (
    <div className="editor-formula-segment">
      <div className="editor-formula-segment-main">
        <div className="editor-formula-kind-wrap editor-formula-kind-wrap--clearable">
          <input className="editor-formula-input editor-formula-input--kind" readOnly aria-label="Action" />
          <button type="button" className="editor-formula-remove-inline" aria-label="Remove step 2">
            <span className="editor-formula-remove-inline-x" />
          </button>
        </div>
      </div>
    </div>
  );
}

describe("CommandFormulaRow pending-step remove placement", () => {
  it("keeps inset remove inside the kind wrap next to the Action field", () => {
    const html = renderToStaticMarkup(<PendingKindRemoveHarness />);
    expect(html).toMatch(
      /<div class="editor-formula-kind-wrap editor-formula-kind-wrap--clearable"[^>]*>[\s\S]*?<button[^>]*class="editor-formula-remove-inline"/,
    );
    expect(html).not.toContain("editor-formula-arg-slot--clearable");
  });
});

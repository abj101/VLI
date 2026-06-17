import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { CommandDraftRow } from "./CommandFormulaRow";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("CommandDraftRow", () => {
  it("shows composer panel before formula row", () => {
    const html = renderToStaticMarkup(
      <CommandDraftRow onDiscard={() => {}} onCreated={() => {}} />,
    );
    expect(html).toContain("editor-composer-panel");
    expect(html).toContain("Trigger");
    expect(html).toContain("What should happen");
    expect(html).toContain("Generate");
    expect(html).not.toContain('aria-label="Save"');
  });
});

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { CommandDraftRow } from "./CommandFormulaRow";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("CommandDraftRow", () => {
  it("labels the primary action Save (not Create command)", () => {
    const html = renderToStaticMarkup(
      <CommandDraftRow onDiscard={() => {}} onCreated={() => {}} />,
    );
    expect(html).toContain(">Save<");
    expect(html).not.toContain("Create command");
    expect(html).toContain('class="editor-settings-primary-btn"');
  });
});

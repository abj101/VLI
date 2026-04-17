import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { CommandDraftRow } from "./CommandFormulaRow";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("CommandDraftRow", () => {
  it("uses ghost icon actions with accessible names", () => {
    const html = renderToStaticMarkup(
      <CommandDraftRow onDiscard={() => {}} onCreated={() => {}} />,
    );
    expect(html).toContain('aria-label="Cancel"');
    expect(html).toContain('aria-label="Save"');
    expect(html).toContain("editor-command-draft-icon-btn");
    expect(html).toContain("editor-command-draft-icon-btn--accent");
    expect(html).not.toContain("Create command");
  });
});

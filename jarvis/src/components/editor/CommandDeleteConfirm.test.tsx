import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { CommandNodePayload } from "../../types";
import { CommandDeleteConfirm } from "./CommandDeleteConfirm";
import { CommandFormulaRow } from "./CommandFormulaRow";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function sampleNode(): CommandNodePayload {
  return {
    id: 12,
    name: "lights",
    trigger_phrases: ["turn on lights"],
    actions: [{ open_url: { url: "https://example.com" } }],
    enabled: true,
    fuzzy_threshold_pct: 0,
    created_at: "2026-04-19T00:00:00Z",
  };
}

describe("CommandDeleteConfirm", () => {
  it("renders idle delete control with phrase-specific label", () => {
    const html = renderToStaticMarkup(
      <CommandDeleteConfirm
        deletePending={false}
        phraseLabel="turn on lights"
        onRequestDelete={() => {}}
        onCancel={() => {}}
        onConfirm={() => {}}
      />,
    );
    expect(html).toContain('aria-label="Delete turn on lights"');
    expect(html).toContain('data-pending="false"');
    expect(html).toMatch(/editor-command-delete-confirm__pending[^>]*aria-hidden="true"/);
  });

  it("renders confirm/cancel icon group when pending", () => {
    const html = renderToStaticMarkup(
      <CommandDeleteConfirm
        deletePending={true}
        phraseLabel="turn on lights"
        onRequestDelete={() => {}}
        onCancel={() => {}}
        onConfirm={() => {}}
      />,
    );
    expect(html).toContain('data-pending="true"');
    expect(html).toContain('aria-label="Confirm delete"');
    expect(html).toContain('aria-label="Cancel delete"');
    expect(html).toContain("editor-command-draft-icon-btn--confirm");
  });
});

describe("CommandFormulaRow delete confirm", () => {
  it("defaults to idle delete control without confirm group visible", () => {
    const html = renderToStaticMarkup(
      <CommandFormulaRow node={sampleNode()} onToggleEnabled={() => {}} onDelete={() => {}} />,
    );
    expect(html).toContain('aria-label="Delete turn on lights"');
    expect(html).toContain('data-pending="false"');
    expect(html).not.toContain("editor-command-item--delete-pending");
  });
});

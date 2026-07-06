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
    expect(html).toContain("editor-composer-textarea");
    expect(html).toContain("Trigger");
    expect(html).toContain("What should happen");
    expect(html).toContain("Generate");
    expect(html).not.toContain('aria-label="Save"');
  });

  it("uses macOS placeholders when composer status reports macos", () => {
    const html = renderToStaticMarkup(
      <CommandDraftRow
        onDiscard={() => {}}
        onCreated={() => {}}
        initialComposerStatus={{
          featureCompiled: true,
          compileBackend: "metal",
          runtimeAvailable: true,
          modelPresent: true,
          modelPath: "/tmp/model.gguf",
          composerEnabled: true,
          ready: true,
          loading: false,
          message: null,
          platform: "macos",
        }}
      />,
    );
    expect(html).toContain("e.g. open TextEdit");
    expect(html).toContain("create a new note, and paste hello world");
    expect(html).not.toContain("e.g. open notepad");
  });
});

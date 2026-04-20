import { describe, expect, it } from "vitest";
import { editorPendingAction } from "../../types";
import {
  appExeDisplayLabel,
  applyRemoveActionAt,
  computeInsetRemoveAllowed,
  deriveAppSearchMeta,
  deriveOpenAppDisplayMode,
  formulaArgInputClass,
} from "./formulaRow.logic";

describe("deriveAppSearchMeta", () => {
  it("shows searching feedback while request is in-flight", () => {
    const meta = deriveAppSearchMeta({
      isOpen: true,
      query: "note",
      isLoading: true,
      hasSearched: false,
      hitCount: 0,
    });
    expect(meta.statusText).toBe("Searching…");
    expect(meta.countText).toBeNull();
  });

  it("shows empty-state feedback when searched query returns no results", () => {
    const meta = deriveAppSearchMeta({
      isOpen: true,
      query: "missing app",
      isLoading: false,
      hasSearched: true,
      hitCount: 0,
      indexCount: 120,
    });
    expect(meta.statusText).toBe('No apps match "missing app"');
  });

  it("shows indexing status when the app index has not been populated yet", () => {
    const pending = deriveAppSearchMeta({
      isOpen: true,
      query: "notepad",
      isLoading: false,
      hasSearched: true,
      hitCount: 0,
      indexCount: null,
    });
    expect(pending.statusText).toBe("Indexing apps…");

    const scanning = deriveAppSearchMeta({
      isOpen: true,
      query: "",
      isLoading: false,
      hasSearched: false,
      hitCount: 0,
      indexCount: 0,
      isScanning: true,
    });
    expect(scanning.statusText).toBe("Indexing apps…");
  });

  it("stops showing indexing status once a scan has completed even if empty", () => {
    const scanDoneButEmpty = deriveAppSearchMeta({
      isOpen: true,
      query: "notepad",
      isLoading: false,
      hasSearched: true,
      hitCount: 0,
      indexCount: 0,
      isScanning: false,
    });
    expect(scanDoneButEmpty.statusText).toBe('No apps match "notepad"');
  });

  it("shows found-count feedback when matches exist", () => {
    const meta = deriveAppSearchMeta({
      isOpen: true,
      query: "calc",
      isLoading: false,
      hasSearched: true,
      hitCount: 3,
    });
    expect(meta.statusText).toBeNull();
    expect(meta.countText).toBe("Found 3 apps");
  });
});

describe("deriveOpenAppDisplayMode", () => {
  it("returns confirmed mode when path exists and edit mode is not active", () => {
    const mode = deriveOpenAppDisplayMode({
      isEditing: false,
      selectedPath: "C:\\Windows\\System32\\notepad.exe",
    });
    expect(mode).toBe("confirmed");
  });

  it("returns edit mode when no selected path exists", () => {
    const mode = deriveOpenAppDisplayMode({
      isEditing: false,
      selectedPath: "",
    });
    expect(mode).toBe("edit");
  });

  it("returns edit mode when user explicitly re-enters edit mode", () => {
    const mode = deriveOpenAppDisplayMode({
      isEditing: true,
      selectedPath: "C:\\Apps\\Discord.exe",
    });
    expect(mode).toBe("edit");
  });
});

describe("computeInsetRemoveAllowed", () => {
  it("allows inset remove for a single concrete action (so hover ✕ is not gated on +)", () => {
    expect(computeInsetRemoveAllowed([{ open_url: { url: "https://a" } }])).toBe(true);
  });

  it("disallows inset remove for a single pending placeholder row", () => {
    expect(computeInsetRemoveAllowed([editorPendingAction()])).toBe(false);
  });

  it("allows inset remove when multiple steps exist", () => {
    expect(
      computeInsetRemoveAllowed([
        { open_url: { url: "https://a" } },
        { wait: { ms: 1 } },
      ]),
    ).toBe(true);
  });
});

describe("applyRemoveActionAt", () => {
  it("replaces the sole concrete step with a pending row instead of dropping to zero actions", () => {
    const sole = { open_url: { url: "https://x" } } as const;
    const next = applyRemoveActionAt([sole], 0);
    expect(next).toHaveLength(1);
    expect(next[0]).toEqual(editorPendingAction());
  });

  it("no-ops remove on sole pending row", () => {
    const pending = editorPendingAction();
    expect(applyRemoveActionAt([pending], 0)).toEqual([pending]);
  });

  it("drops one row when multiple actions exist", () => {
    const a = { wait: { ms: 1 } } as const;
    const b = { wait: { ms: 2 } } as const;
    expect(applyRemoveActionAt([a, b], 0)).toEqual([b]);
  });
});

describe("appExeDisplayLabel", () => {
  it("returns the file name for a Windows path", () => {
    expect(appExeDisplayLabel(String.raw`C:\Apps\Discord\Discord.exe`)).toBe("Discord.exe");
  });

  it("returns the last segment for shell app targets", () => {
    expect(
      appExeDisplayLabel(String.raw`shell:AppsFolder\com.squirrel.Discord.Discord`),
    ).toBe("com.squirrel.Discord.Discord");
  });

  it("returns host-style path after the protocol for steam://", () => {
    expect(appExeDisplayLabel("steam://rungameid/730")).toBe("rungameid/730");
  });
});

describe("formulaArgInputClass", () => {
  it("includes autogrow class for regular text boxes", () => {
    expect(formulaArgInputClass()).toContain("editor-formula-input--autogrow");
  });

  it("keeps narrow class without autogrow for numeric narrow inputs", () => {
    const klass = formulaArgInputClass({ narrow: true, autoGrow: false });
    expect(klass).toContain("editor-formula-input--narrow");
    expect(klass).not.toContain("editor-formula-input--autogrow");
  });
});

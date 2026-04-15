import { describe, expect, it } from "vitest";
import { deriveAppSearchMeta, deriveOpenAppDisplayMode } from "./formulaRow.logic";

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
    });
    expect(meta.statusText).toBe("No apps found");
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

import { describe, expect, it } from "vitest";
import { deriveAppSearchMeta } from "./formulaRow.logic";

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

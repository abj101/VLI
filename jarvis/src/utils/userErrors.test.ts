import { describe, expect, it } from "vitest";
import { formatUserError } from "./userErrors";

describe("formatUserError", () => {
  it("returns fallback for empty unknown", () => {
    expect(formatUserError("", "Save failed.")).toBe("Save failed.");
  });

  it("maps connection errors", () => {
    expect(formatUserError(new Error("ECONNREFUSED"), "x")).toMatch(/connection/i);
  });

  it("maps timeout", () => {
    expect(formatUserError(new Error("Request timed out"), "x")).toMatch(/timed out/i);
  });

  it("passes through short user-ish messages", () => {
    expect(formatUserError(new Error("Hotkey already in use"), "x")).toBe("Hotkey already in use");
  });
});

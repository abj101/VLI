import { describe, expect, it } from "vitest";
import {
  normalizeThemeValue,
  parseThresholdSettingValue,
  validateHotkeyInput,
} from "./SettingsPanel.logic";

describe("SettingsPanel logic", () => {
  it("parses threshold setting only inside supported range", () => {
    expect(parseThresholdSettingValue("80")).toBe(0.8);
    expect(parseThresholdSettingValue("50")).toBe(0.5);
    expect(parseThresholdSettingValue("100")).toBe(1);
    expect(parseThresholdSettingValue("49")).toBeNull();
    expect(parseThresholdSettingValue("abc")).toBeNull();
  });

  it("normalizes theme values and falls back to dark", () => {
    expect(normalizeThemeValue("dark")).toBe("dark");
    expect(normalizeThemeValue("light")).toBe("light");
    expect(normalizeThemeValue("unknown")).toBe("dark");
  });

  it("requires non-empty hotkey input", () => {
    expect(validateHotkeyInput("ctrl+shift+j")).toBeNull();
    expect(validateHotkeyInput("   ")).toBe("Hotkey is required.");
  });
});

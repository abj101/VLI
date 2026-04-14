import { describe, expect, it } from "vitest";
import {
  normalizeSttProvider,
  normalizeThemeValue,
  parseRemoteSttTimeoutSecs,
  parseThresholdSettingValue,
  validateHotkeyInput,
} from "./SettingsPanel.logic";

describe("SettingsPanel logic", () => {
  it("normalizes STT provider to local, os, or remote", () => {
    expect(normalizeSttProvider(null)).toBe("local");
    expect(normalizeSttProvider(undefined)).toBe("local");
    expect(normalizeSttProvider("")).toBe("local");
    expect(normalizeSttProvider("LOCAL")).toBe("local");
    expect(normalizeSttProvider("os")).toBe("os");
    expect(normalizeSttProvider("Remote")).toBe("remote");
    expect(normalizeSttProvider("bogus")).toBe("local");
  });

  it("parses remote STT timeout only inside 1–300", () => {
    expect(parseRemoteSttTimeoutSecs("30")).toBe(30);
    expect(parseRemoteSttTimeoutSecs("  1  ")).toBe(1);
    expect(parseRemoteSttTimeoutSecs("300")).toBe(300);
    expect(parseRemoteSttTimeoutSecs("0")).toBeNull();
    expect(parseRemoteSttTimeoutSecs("301")).toBeNull();
    expect(parseRemoteSttTimeoutSecs("")).toBeNull();
    expect(parseRemoteSttTimeoutSecs("abc")).toBeNull();
  });

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

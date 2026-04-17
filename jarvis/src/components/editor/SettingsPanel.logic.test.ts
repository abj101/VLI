import { describe, expect, it, vi } from "vitest";
import {
  normalizeSttProvider,
  normalizeThemePreference,
  parseRemoteSttTimeoutSecs,
  parseThresholdSettingValue,
  resolveEditorTheme,
  shouldWarmupWhisperGpu,
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

  it("normalizes theme preference including system default for unknown", () => {
    expect(normalizeThemePreference("dark")).toBe("dark");
    expect(normalizeThemePreference("light")).toBe("light");
    expect(normalizeThemePreference("system")).toBe("system");
    expect(normalizeThemePreference("SYSTEM")).toBe("system");
    expect(normalizeThemePreference(null)).toBe("system");
    expect(normalizeThemePreference("unknown")).toBe("system");
  });

  it("resolves fixed light and dark preferences", () => {
    expect(resolveEditorTheme("light")).toBe("light");
    expect(resolveEditorTheme("dark")).toBe("dark");
  });

  it("resolves system to light when OS prefers light", () => {
    vi.stubGlobal(
      "matchMedia",
      vi.fn().mockReturnValue({
        matches: true,
        media: "(prefers-color-scheme: light)",
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    );
    expect(resolveEditorTheme("system")).toBe("light");
    vi.unstubAllGlobals();
  });

  it("resolves system to dark when OS prefers dark", () => {
    vi.stubGlobal(
      "matchMedia",
      vi.fn().mockReturnValue({
        matches: false,
        media: "(prefers-color-scheme: light)",
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    );
    expect(resolveEditorTheme("system")).toBe("dark");
    vi.unstubAllGlobals();
  });

  it("requires non-empty hotkey input", () => {
    expect(validateHotkeyInput("ctrl+shift+j")).toBeNull();
    expect(validateHotkeyInput("   ")).toBe("Hotkey is required.");
  });

  it("warms up Vulkan GPU model only when enabling supported Vulkan", () => {
    expect(
      shouldWarmupWhisperGpu({
        nextEnabled: true,
        compileBackend: "vulkan",
        runtimeAvailable: true,
      }),
    ).toBe(true);
    expect(
      shouldWarmupWhisperGpu({
        nextEnabled: false,
        compileBackend: "vulkan",
        runtimeAvailable: true,
      }),
    ).toBe(false);
    expect(
      shouldWarmupWhisperGpu({
        nextEnabled: true,
        compileBackend: "cuda",
        runtimeAvailable: true,
      }),
    ).toBe(false);
    expect(
      shouldWarmupWhisperGpu({
        nextEnabled: true,
        compileBackend: "vulkan",
        runtimeAvailable: false,
      }),
    ).toBe(false);
  });
});

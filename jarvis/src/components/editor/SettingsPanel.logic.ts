export type EditorThemePreference = "dark" | "light" | "system";

/** Resolved palette applied to `data-theme` (always light or dark). */
export type ResolvedEditorTheme = "dark" | "light";

/** Matches `stt_provider` in SQLite / `AppSettings.stt_provider`. */
export type SttProvider = "local" | "os" | "remote";

export function normalizeSttProvider(raw: string | null | undefined): SttProvider {
  const s = (raw ?? "").trim().toLowerCase();
  if (s === "os" || s === "remote") return s;
  return "local";
}

/** Parses remote STT timeout for settings UI; valid range 1–300 seconds. */
export function parseRemoteSttTimeoutSecs(raw: string): number | null {
  const n = Number.parseInt(raw.trim(), 10);
  if (!Number.isFinite(n) || n < 1 || n > 300) {
    return null;
  }
  return n;
}

export function parseThresholdSettingValue(raw: string | null): number | null {
  if (!raw) return null;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed < 50 || parsed > 100) {
    return null;
  }
  return parsed / 100;
}

/**
 * Normalizes persisted `theme` setting.
 * Unknown / empty → `system` (follow OS).
 */
export function normalizeThemePreference(raw: string | null | undefined): EditorThemePreference {
  const s = (raw ?? "").trim().toLowerCase();
  if (s === "light") return "light";
  if (s === "dark") return "dark";
  if (s === "system") return "system";
  return "system";
}

export function resolveEditorTheme(pref: EditorThemePreference): ResolvedEditorTheme {
  if (pref === "light") return "light";
  if (pref === "dark") return "dark";
  if (typeof globalThis.matchMedia !== "function") return "dark";
  return globalThis.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

/** Sets `data-theme` (resolved) and `data-theme-preference` on `<html>`. */
export function applyEditorThemeToDocument(pref: EditorThemePreference): void {
  if (typeof document === "undefined") return;
  const resolved = resolveEditorTheme(pref);
  document.documentElement.setAttribute("data-theme-preference", pref);
  document.documentElement.setAttribute("data-theme", resolved);
}

export function validateHotkeyInput(raw: string): string | null {
  return raw.trim().length > 0 ? null : "Hotkey is required.";
}

type WhisperGpuWarmupCheck = {
  nextEnabled: boolean;
  compileBackend: string;
  runtimeAvailable: boolean;
};

export function shouldWarmupWhisperGpu({
  nextEnabled,
  compileBackend,
  runtimeAvailable,
}: WhisperGpuWarmupCheck): boolean {
  return nextEnabled && runtimeAvailable && compileBackend === "vulkan";
}

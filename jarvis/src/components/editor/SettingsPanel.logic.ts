export type EditorTheme = "dark" | "light";

export function parseThresholdSettingValue(raw: string | null): number | null {
  if (!raw) return null;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed < 50 || parsed > 100) {
    return null;
  }
  return parsed / 100;
}

export function normalizeThemeValue(raw: string | null): EditorTheme {
  return raw === "light" ? "light" : "dark";
}

export function validateHotkeyInput(raw: string): string | null {
  return raw.trim().length > 0 ? null : "Hotkey is required.";
}

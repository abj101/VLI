export type EditorTheme = "dark" | "light";

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

export function normalizeThemeValue(raw: string | null): EditorTheme {
  return raw === "light" ? "light" : "dark";
}

export function validateHotkeyInput(raw: string): string | null {
  return raw.trim().length > 0 ? null : "Hotkey is required.";
}

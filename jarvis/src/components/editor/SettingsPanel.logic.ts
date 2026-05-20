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

/**
 * Match a Tauri/global-hotkey style chord (e.g. `escape`, `ctrl+shift+j`) against
 * a `keydown` event. Used when the HUD webview has focus as a fallback alongside
 * the native global shortcut.
 */
export function hotkeyChordMatchesKeyboardEvent(raw: string, e: KeyboardEvent): boolean {
  const segments = raw
    .split("+")
    .map((s) => s.trim().toLowerCase())
    .filter(Boolean);
  if (segments.length === 0) return false;

  let wantCtrl = false;
  let wantShift = false;
  let wantAlt = false;
  let wantMeta = false;
  const keyParts: string[] = [];

  for (const seg of segments) {
    if (seg === "ctrl" || seg === "control") wantCtrl = true;
    else if (seg === "shift") wantShift = true;
    else if (seg === "alt" || seg === "option") wantAlt = true;
    else if (seg === "meta" || seg === "cmd" || seg === "command" || seg === "super") wantMeta = true;
    else keyParts.push(seg);
  }

  if (keyParts.length !== 1) return false;

  if (
    (e.ctrlKey ?? false) !== wantCtrl ||
    (e.shiftKey ?? false) !== wantShift ||
    (e.altKey ?? false) !== wantAlt ||
    (e.metaKey ?? false) !== wantMeta
  ) {
    return false;
  }

  const wantKey = normalizeHotkeyKeyToken(keyParts[0]);
  const pressed = keyboardEventKeyToken(e);
  if (!pressed) return false;
  return pressed === wantKey;
}

function normalizeHotkeyKeyToken(s: string): string {
  const t = s.trim().toLowerCase();
  if (t === "esc") return "escape";
  if (t === "return" || t === "enter") return "enter";
  if (t === "space" || t === "spc") return "space";
  if (t === "down") return "arrowdown";
  if (t === "up") return "arrowup";
  if (t === "left") return "arrowleft";
  if (t === "right") return "arrowright";
  return t;
}

function keyboardEventKeyToken(e: KeyboardEvent): string {
  const k = e.key;
  if (k === " ") return "space";
  const lower = k.toLowerCase();
  if (
    lower === "control" ||
    lower === "shift" ||
    lower === "alt" ||
    lower === "meta" ||
    lower === "os" ||
    lower === "osleft" ||
    lower === "osright"
  ) {
    return "";
  }
  if (lower === "escape") return "escape";
  if (lower === "enter") return "enter";
  if (lower.length === 1) return lower;
  if (/^f\d{1,2}$/.test(lower)) return lower;
  if (lower.startsWith("arrow")) return lower;
  return lower;
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

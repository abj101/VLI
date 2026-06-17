export type EditorThemePreference = "dark" | "light" | "system";

/** Resolved palette applied to `data-theme` (always light or dark). */
export type ResolvedEditorTheme = "dark" | "light";

/** Matches `stt_provider` in SQLite / `AppSettings.stt_provider`. */
export type SttProvider = "local" | "os" | "remote";

/** Matches `local_whisper_model` in SQLite / `AppSettings.localWhisperModel`. */
export type LocalWhisperModelId = "tiny.en" | "base.en" | "small.en";

export const LOCAL_WHISPER_MODEL_OPTIONS: ReadonlyArray<{
  value: LocalWhisperModelId;
  label: string;
}> = [
  { value: "tiny.en", label: "Tiny — fastest, least accurate" },
  { value: "base.en", label: "Base — balanced (recommended)" },
  { value: "small.en", label: "Small — most accurate, slowest" },
] as const;

export function normalizeLocalWhisperModel(
  raw: string | null | undefined,
): LocalWhisperModelId {
  const s = (raw ?? "").trim();
  if (s === "base.en" || s === "small.en") return s;
  return "tiny.en";
}

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

/** Persisted `hud_transparency` setting: 0 = theme CSS default, 100 = most transparent. */
export const HUD_TRANSPARENCY_MIN = 0;
export const HUD_TRANSPARENCY_MAX = 100;
export const HUD_TRANSPARENCY_DEFAULT = 50;
export const HUD_MATERIAL_ALPHA_FLOOR_PCT = 50;

const HUD_MATERIAL_ALPHA_DEFAULT: Record<ResolvedEditorTheme, number> = {
  light: 78,
  dark: 88,
};

export function parseHudTransparencySettingValue(raw: string | null | undefined): number {
  if (!raw || !raw.trim()) return HUD_TRANSPARENCY_DEFAULT;
  const parsed = Number.parseInt(raw.trim(), 10);
  if (!Number.isFinite(parsed)) return HUD_TRANSPARENCY_DEFAULT;
  return Math.max(HUD_TRANSPARENCY_MIN, Math.min(HUD_TRANSPARENCY_MAX, parsed));
}

/** Maps transparency (0–100) to HUD glass fill alpha, or `null` for theme default. */
export function resolveHudMaterialAlphaPct(
  transparency: number,
  theme: ResolvedEditorTheme,
): number | null {
  const clamped = Math.max(
    HUD_TRANSPARENCY_MIN,
    Math.min(HUD_TRANSPARENCY_MAX, transparency),
  );
  if (clamped <= HUD_TRANSPARENCY_MIN) return null;
  const defaultAlpha = HUD_MATERIAL_ALPHA_DEFAULT[theme];
  const t = clamped / HUD_TRANSPARENCY_MAX;
  const alpha = Math.round(defaultAlpha - t * (defaultAlpha - HUD_MATERIAL_ALPHA_FLOOR_PCT));
  return Math.max(HUD_MATERIAL_ALPHA_FLOOR_PCT, alpha);
}

function readThemePreferenceFromDocument(): EditorThemePreference {
  if (typeof document === "undefined") return "system";
  const pref = document.documentElement.getAttribute("data-theme-preference");
  return normalizeThemePreference(pref);
}

/** Applies HUD glass opacity override on `<html>` (`--hud-material-alpha`). */
export function applyHudTransparencyToDocument(
  transparency: number,
  themePref?: EditorThemePreference,
): void {
  if (typeof document === "undefined") return;
  const pref = themePref ?? readThemePreferenceFromDocument();
  const theme = resolveEditorTheme(pref);
  const alpha = resolveHudMaterialAlphaPct(transparency, theme);
  if (alpha === null) {
    document.documentElement.style.removeProperty("--hud-material-alpha");
    return;
  }
  document.documentElement.style.setProperty("--hud-material-alpha", `${alpha}%`);
}

/** Persisted `editor_transparency` setting: 0 = opaque, 100 = most transparent. */
export const EDITOR_TRANSPARENCY_MIN = 0;
export const EDITOR_TRANSPARENCY_MAX = 100;
export const EDITOR_TRANSPARENCY_DEFAULT = 50;
export const EDITOR_MATERIAL_ALPHA_MAX_PCT = 100;
export const EDITOR_MATERIAL_ALPHA_FLOOR_PCT = 75;

const EDITOR_MATERIAL_ALPHA_DEFAULT: Record<ResolvedEditorTheme, number> = {
  light: 92,
  dark: 95,
};

const EDITOR_SIDEBAR_MATERIAL_ALPHA_DEFAULT: Record<ResolvedEditorTheme, number> = {
  light: 97,
  dark: 98,
};

function editorSidebarAlphaOffset(theme: ResolvedEditorTheme): number {
  return (
    EDITOR_SIDEBAR_MATERIAL_ALPHA_DEFAULT[theme] - EDITOR_MATERIAL_ALPHA_DEFAULT[theme]
  );
}

export function parseEditorTransparencySettingValue(raw: string | null | undefined): number {
  if (!raw || !raw.trim()) return EDITOR_TRANSPARENCY_DEFAULT;
  const parsed = Number.parseInt(raw.trim(), 10);
  if (!Number.isFinite(parsed)) return EDITOR_TRANSPARENCY_DEFAULT;
  return Math.max(EDITOR_TRANSPARENCY_MIN, Math.min(EDITOR_TRANSPARENCY_MAX, parsed));
}

/** Maps transparency (0–100) to editor shell glass fill alpha (100% at 0, floor at 100). */
export function resolveEditorMaterialAlphaPct(
  transparency: number,
  _theme: ResolvedEditorTheme,
): number {
  const clamped = Math.max(
    EDITOR_TRANSPARENCY_MIN,
    Math.min(EDITOR_TRANSPARENCY_MAX, transparency),
  );
  const t = clamped / EDITOR_TRANSPARENCY_MAX;
  const alpha = Math.round(
    EDITOR_MATERIAL_ALPHA_MAX_PCT -
      t * (EDITOR_MATERIAL_ALPHA_MAX_PCT - EDITOR_MATERIAL_ALPHA_FLOOR_PCT),
  );
  return Math.max(EDITOR_MATERIAL_ALPHA_FLOOR_PCT, alpha);
}

/** Sidebar tracks shell alpha with the same theme default offset (light +5, dark +3). */
export function resolveEditorSidebarMaterialAlphaPct(
  transparency: number,
  theme: ResolvedEditorTheme,
): number {
  const shellAlpha = resolveEditorMaterialAlphaPct(transparency, theme);
  return Math.min(100, shellAlpha + editorSidebarAlphaOffset(theme));
}

/** Applies editor glass opacity overrides on `<html>`. */
export function applyEditorTransparencyToDocument(
  transparency: number,
  themePref?: EditorThemePreference,
): void {
  if (typeof document === "undefined") return;
  const pref = themePref ?? readThemePreferenceFromDocument();
  const theme = resolveEditorTheme(pref);
  const shellAlpha = resolveEditorMaterialAlphaPct(transparency, theme);
  const sidebarAlpha = resolveEditorSidebarMaterialAlphaPct(transparency, theme);
  document.documentElement.style.setProperty("--editor-material-alpha", `${shellAlpha}%`);
  document.documentElement.style.setProperty(
    "--editor-sidebar-material-alpha",
    `${sidebarAlpha}%`,
  );
}

export function validateHotkeyInput(raw: string): string | null {
  return raw.trim().length > 0 ? null : "Hotkey is required.";
}

export function parseHotkeyChord(raw: string): string[] {
  const trimmed = raw.trim();
  if (!trimmed) return [];
  return trimmed
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);
}

export function isHotkeyModifierToken(token: string): boolean {
  const t = token.trim().toLowerCase();
  return (
    t === "ctrl" ||
    t === "control" ||
    t === "shift" ||
    t === "alt" ||
    t === "meta" ||
    t === "super" ||
    t === "win"
  );
}

export function formatHotkeyKeyLabel(token: string): string {
  const t = token.trim().toLowerCase();
  switch (t) {
    case "ctrl":
    case "control":
      return "Ctrl";
    case "shift":
      return "Shift";
    case "alt":
      return "Alt";
    case "meta":
    case "super":
    case "win":
      return "Win";
    case "escape":
      return "Esc";
    case " ":
    case "space":
      return "Space";
    case "arrowup":
      return "↑";
    case "arrowdown":
      return "↓";
    case "arrowleft":
      return "←";
    case "arrowright":
      return "→";
    default:
      if (t.length === 1) return t.toUpperCase();
      return token.trim().charAt(0).toUpperCase() + token.trim().slice(1).toLowerCase();
  }
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

export type HotkeyModifierToken = "ctrl" | "alt" | "shift" | "meta";

export type HotkeyRecordingState = {
  /** Modifiers seen during this recording session (keydown adds, keyup removes). */
  heldModifiers: Set<HotkeyModifierToken>;
};

export function createHotkeyRecordingState(): HotkeyRecordingState {
  return { heldModifiers: new Set() };
}

export type HotkeyRecordingAction =
  | { type: "ignore" }
  | { type: "cancel" }
  | { type: "clear" }
  | { type: "record"; chord: string };

/** Skip auto-repeat and IME composition keydowns during shortcut capture. */
export function isIgnorableHotkeyRecordingEvent(e: KeyboardEvent): boolean {
  if (e.repeat) return true;
  if (e.isComposing || e.keyCode === 229) return true;
  return false;
}

function hotkeyRecordingEventHasModifierFlags(e: KeyboardEvent): boolean {
  return Boolean(e.ctrlKey || e.shiftKey || e.altKey || e.metaKey);
}

export function hotkeyModifierTokenFromKey(key: string): HotkeyModifierToken | null {
  const lower = key.toLowerCase();
  if (lower === "control") return "ctrl";
  if (lower === "shift") return "shift";
  if (lower === "alt" || lower === "option" || lower === "altgraph") return "alt";
  if (lower === "meta" || lower === "os" || lower === "osleft" || lower === "osright") return "meta";
  return null;
}

export function hotkeyModifierTokenFromCode(code: string): HotkeyModifierToken | null {
  if (code.startsWith("Control")) return "ctrl";
  if (code.startsWith("Shift")) return "shift";
  if (code.startsWith("Alt")) return "alt";
  if (code.startsWith("Meta")) return "meta";
  return null;
}

export function hotkeyModifierTokenFromEvent(e: KeyboardEvent): HotkeyModifierToken | null {
  return hotkeyModifierTokenFromKey(e.key) ?? hotkeyModifierTokenFromCode(e.code);
}

function mergeEventModifiers(e: KeyboardEvent, mods: Set<HotkeyModifierToken>): void {
  const modifierState =
    typeof e.getModifierState === "function"
      ? (mod: string) => e.getModifierState(mod)
      : () => false;
  if (e.ctrlKey || modifierState("Control")) mods.add("ctrl");
  if (e.altKey || modifierState("Alt")) mods.add("alt");
  if (e.shiftKey || modifierState("Shift")) mods.add("shift");
  if (e.metaKey || modifierState("Meta")) mods.add("meta");
}

export function collectHotkeyModifiers(
  e: KeyboardEvent,
  held: Iterable<HotkeyModifierToken>,
): Set<HotkeyModifierToken> {
  const mods = new Set(held);
  mergeEventModifiers(e, mods);
  return mods;
}

export function formatHotkeyChord(mods: Iterable<HotkeyModifierToken>, keyToken: string): string {
  const set = mods instanceof Set ? mods : new Set(mods);
  const parts: string[] = [];
  if (set.has("ctrl")) parts.push("ctrl");
  if (set.has("alt")) parts.push("alt");
  if (set.has("shift")) parts.push("shift");
  if (set.has("meta")) parts.push("meta");
  parts.push(keyToken);
  return parts.join("+");
}

/**
 * Advance hotkey recording on `keydown`. Modifier-only presses wait for a main key; the final
 * chord uses modifiers held at keydown time (event flags + session-held modifiers).
 */
export function processHotkeyRecordingKeyDown(
  e: KeyboardEvent,
  state: HotkeyRecordingState,
): { next: HotkeyRecordingState; action: HotkeyRecordingAction } {
  const next: HotkeyRecordingState = {
    heldModifiers: new Set(state.heldModifiers),
  };

  if (isIgnorableHotkeyRecordingEvent(e)) {
    return { next, action: { type: "ignore" } };
  }

  if (e.key === "Escape") {
    return { next, action: { type: "cancel" } };
  }

  if (
    (e.key === "Backspace" || e.key === "Delete") &&
    !hotkeyRecordingEventHasModifierFlags(e)
  ) {
    return { next, action: { type: "clear" } };
  }

  const modToken = hotkeyModifierTokenFromEvent(e);
  if (modToken) {
    next.heldModifiers.add(modToken);
    mergeEventModifiers(e, next.heldModifiers);
    return { next, action: { type: "ignore" } };
  }

  const keyToken = keyboardEventKeyToken(e);
  if (!keyToken) {
    return { next, action: { type: "ignore" } };
  }

  const mods = collectHotkeyModifiers(e, next.heldModifiers);
  return {
    next,
    action: { type: "record", chord: formatHotkeyChord(mods, keyToken) },
  };
}

/** Drop released modifiers so a later main key does not inherit stale holds. */
export function processHotkeyRecordingKeyUp(
  e: KeyboardEvent,
  state: HotkeyRecordingState,
): HotkeyRecordingState {
  const modToken = hotkeyModifierTokenFromEvent(e);
  if (!modToken) return state;
  const next: HotkeyRecordingState = {
    heldModifiers: new Set(state.heldModifiers),
  };
  next.heldModifiers.delete(modToken);
  return next;
}

/**
 * Build a Tauri/global-hotkey style chord string from a `keydown` event.
 * Returns `null` for modifier-only presses.
 */
export function keyboardEventToHotkeyChord(e: KeyboardEvent): string | null {
  const keyToken = keyboardEventKeyToken(e);
  if (!keyToken) return null;
  const mods = collectHotkeyModifiers(e, []);
  return formatHotkeyChord(mods, keyToken);
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

export const SETTINGS_NOTICE_AUTO_DISMISS_MS = 10_000;
export const SETTINGS_NOTICE_FADE_MS = 320;

export type SettingsNoticePhase = "hidden" | "visible" | "hiding";

type NoticeTimeout = ReturnType<typeof setTimeout>;

/** Starts a settings footer notice auto-dismiss timer; returns cancel. */
export function scheduleSettingsNoticeAutoDismiss(
  onDismiss: () => void,
  ms: number = SETTINGS_NOTICE_AUTO_DISMISS_MS,
  setTimer: (fn: () => void, delay: number) => NoticeTimeout = setTimeout,
  clearTimer: (id: NoticeTimeout) => void = clearTimeout,
): () => void {
  const id = setTimer(() => {
    onDismiss();
  }, ms);
  return () => clearTimer(id);
}

/** Starts the fade-out tail after auto-dismiss; returns cancel. */
export function scheduleSettingsNoticeFadeOut(
  onComplete: () => void,
  ms: number = SETTINGS_NOTICE_FADE_MS,
  setTimer: (fn: () => void, delay: number) => NoticeTimeout = setTimeout,
  clearTimer: (id: NoticeTimeout) => void = clearTimeout,
): () => void {
  const id = setTimer(() => {
    onComplete();
  }, ms);
  return () => clearTimer(id);
}

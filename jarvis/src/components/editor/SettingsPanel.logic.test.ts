import { describe, expect, it, vi } from "vitest";
import {
  hotkeyChordMatchesKeyboardEvent,
  keyboardEventToHotkeyChord,
  createHotkeyRecordingState,
  isIgnorableHotkeyRecordingEvent,
  processHotkeyRecordingKeyDown,
  processHotkeyRecordingKeyUp,
  normalizeSttProvider,
  normalizeLocalWhisperModel,
  DEFAULT_LOCAL_WHISPER_MODEL,
  formatByteSize,
  whisperModelShortTitle,
  whisperModelSubtitle,
  normalizeDictationHotkeyMode,
  normalizeThemePreference,
  parseEditorTransparencySettingValue,
  parseHudTransparencySettingValue,
  parseRemoteSttTimeoutSecs,
  parseThresholdSettingValue,
  resolveEditorMaterialAlphaPct,
  resolveEditorSidebarMaterialAlphaPct,
  resolveHudMaterialAlphaPct,
  resolveEditorTheme,
  scheduleSettingsNoticeAutoDismiss,
  scheduleSettingsNoticeFadeOut,
  SETTINGS_NOTICE_AUTO_DISMISS_MS,
  SETTINGS_NOTICE_FADE_MS,
  shouldWarmupWhisperGpu,
  validateHotkeyInput,
  parseHotkeyChord,
  formatHotkeyKeyLabel,
  isHotkeyModifierToken,
} from "./SettingsPanel.logic";

describe("SettingsPanel logic", () => {
  it("normalizes local Whisper model id with base.en default", () => {
    expect(normalizeLocalWhisperModel(null)).toBe(DEFAULT_LOCAL_WHISPER_MODEL);
    expect(normalizeLocalWhisperModel(undefined)).toBe("base.en");
    expect(normalizeLocalWhisperModel("")).toBe("base.en");
    expect(normalizeLocalWhisperModel("small.en")).toBe("small.en");
    expect(normalizeLocalWhisperModel("tiny.en")).toBe("tiny.en");
    expect(normalizeLocalWhisperModel("bogus")).toBe("base.en");
  });

  it("formats byte sizes for model list UI", () => {
    expect(formatByteSize(null)).toBe("—");
    expect(formatByteSize(512)).toBe("512 B");
    expect(formatByteSize(1024 * 1024)).toBe("1.0 MB");
  });

  it("splits whisper model labels for card UI", () => {
    expect(whisperModelShortTitle("base.en")).toBe("Base");
    expect(whisperModelSubtitle("base.en")).toBe("balanced (default)");
    expect(whisperModelShortTitle("small.en")).toBe("Small");
  });

  it("normalizes STT provider to local, os, or remote", () => {
    expect(normalizeSttProvider(null)).toBe("local");
    expect(normalizeSttProvider(undefined)).toBe("local");
    expect(normalizeSttProvider("")).toBe("local");
    expect(normalizeSttProvider("LOCAL")).toBe("local");
    expect(normalizeSttProvider("os")).toBe("os");
    expect(normalizeSttProvider("Remote")).toBe("remote");
    expect(normalizeSttProvider("bogus")).toBe("local");
  });

  it("normalizes dictation hotkey mode to toggle or push_to_talk", () => {
    expect(normalizeDictationHotkeyMode(null)).toBe("toggle");
    expect(normalizeDictationHotkeyMode(undefined)).toBe("toggle");
    expect(normalizeDictationHotkeyMode("")).toBe("toggle");
    expect(normalizeDictationHotkeyMode("toggle")).toBe("toggle");
    expect(normalizeDictationHotkeyMode("TOGGLE")).toBe("toggle");
    expect(normalizeDictationHotkeyMode("push_to_talk")).toBe("push_to_talk");
    expect(normalizeDictationHotkeyMode("push-to-talk")).toBe("push_to_talk");
    expect(normalizeDictationHotkeyMode("ptt")).toBe("push_to_talk");
    expect(normalizeDictationHotkeyMode("unknown")).toBe("toggle");
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

  it("parses HUD transparency as 0–100 with unset/invalid → 50", () => {
    expect(parseHudTransparencySettingValue(null)).toBe(50);
    expect(parseHudTransparencySettingValue("")).toBe(50);
    expect(parseHudTransparencySettingValue("0")).toBe(0);
    expect(parseHudTransparencySettingValue("42")).toBe(42);
    expect(parseHudTransparencySettingValue("50")).toBe(50);
    expect(parseHudTransparencySettingValue("100")).toBe(100);
    expect(parseHudTransparencySettingValue("-5")).toBe(0);
    expect(parseHudTransparencySettingValue("150")).toBe(100);
    expect(parseHudTransparencySettingValue("abc")).toBe(50);
  });

  it("resolves HUD material alpha from transparency with theme defaults at 0", () => {
    expect(resolveHudMaterialAlphaPct(0, "light")).toBeNull();
    expect(resolveHudMaterialAlphaPct(0, "dark")).toBeNull();
    expect(resolveHudMaterialAlphaPct(100, "light")).toBe(50);
    expect(resolveHudMaterialAlphaPct(100, "dark")).toBe(50);
    expect(resolveHudMaterialAlphaPct(50, "dark")).toBe(69);
  });

  it("parses editor transparency as 0–100 with unset/invalid → 50", () => {
    expect(parseEditorTransparencySettingValue(null)).toBe(50);
    expect(parseEditorTransparencySettingValue("")).toBe(50);
    expect(parseEditorTransparencySettingValue("0")).toBe(0);
    expect(parseEditorTransparencySettingValue("42")).toBe(42);
    expect(parseEditorTransparencySettingValue("100")).toBe(100);
    expect(parseEditorTransparencySettingValue("-5")).toBe(0);
    expect(parseEditorTransparencySettingValue("150")).toBe(100);
    expect(parseEditorTransparencySettingValue("abc")).toBe(50);
  });

  it("resolves editor material alpha from transparency (0 = opaque, 100 = floor)", () => {
    expect(resolveEditorMaterialAlphaPct(0, "light")).toBe(100);
    expect(resolveEditorMaterialAlphaPct(0, "dark")).toBe(100);
    expect(resolveEditorMaterialAlphaPct(100, "light")).toBe(75);
    expect(resolveEditorMaterialAlphaPct(100, "dark")).toBe(75);
    expect(resolveEditorMaterialAlphaPct(50, "dark")).toBe(88);
    expect(resolveEditorSidebarMaterialAlphaPct(0, "light")).toBe(100);
    expect(resolveEditorSidebarMaterialAlphaPct(0, "dark")).toBe(100);
    expect(resolveEditorSidebarMaterialAlphaPct(100, "light")).toBe(80);
    expect(resolveEditorSidebarMaterialAlphaPct(100, "dark")).toBe(78);
    expect(resolveEditorSidebarMaterialAlphaPct(50, "dark")).toBe(91);
    expect(resolveEditorSidebarMaterialAlphaPct(50, "light")).toBe(93);
  });

  it("keeps sidebar denser than shell by the theme default offset when headroom allows", () => {
    for (const theme of ["light", "dark"] as const) {
      const offset =
        (theme === "light" ? 97 : 98) - (theme === "light" ? 92 : 95);
      for (const transparency of [25, 50, 75, 100]) {
        const shell = resolveEditorMaterialAlphaPct(transparency, theme);
        const sidebar = resolveEditorSidebarMaterialAlphaPct(transparency, theme);
        expect(sidebar).toBe(Math.min(100, shell + offset));
      }
    }
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

  it("matches hotkey chord against keyboard events", () => {
    const esc = {
      key: "Escape",
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    } as KeyboardEvent;
    expect(hotkeyChordMatchesKeyboardEvent("escape", esc)).toBe(true);
    expect(hotkeyChordMatchesKeyboardEvent("Escape", esc)).toBe(true);

    const j = {
      key: "j",
      ctrlKey: true,
      shiftKey: true,
      altKey: false,
      metaKey: false,
    } as KeyboardEvent;
    expect(hotkeyChordMatchesKeyboardEvent("ctrl+shift+j", j)).toBe(true);
    expect(hotkeyChordMatchesKeyboardEvent("ctrl+shift+J", j)).toBe(true);

    const jNoMod = {
      key: "j",
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    } as KeyboardEvent;
    expect(hotkeyChordMatchesKeyboardEvent("ctrl+shift+j", jNoMod)).toBe(false);

    const escWithCtrl = {
      key: "Escape",
      ctrlKey: true,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    } as KeyboardEvent;
    expect(hotkeyChordMatchesKeyboardEvent("escape", escWithCtrl)).toBe(false);
  });

  it("requires non-empty hotkey input", () => {
    expect(validateHotkeyInput("ctrl+shift+j")).toBeNull();
    expect(validateHotkeyInput("   ")).toBe("Hotkey is required.");
  });

  it("parses and formats hotkey chord tokens for display", () => {
    expect(parseHotkeyChord("ctrl+shift+j")).toEqual(["ctrl", "shift", "j"]);
    expect(parseHotkeyChord("  ")).toEqual([]);
    expect(formatHotkeyKeyLabel("ctrl")).toBe("Ctrl");
    expect(formatHotkeyKeyLabel("j")).toBe("J");
    expect(formatHotkeyKeyLabel("escape")).toBe("Esc");
    expect(isHotkeyModifierToken("shift")).toBe(true);
    expect(isHotkeyModifierToken("j")).toBe(false);
  });

  it("builds hotkey chord strings from keyboard events", () => {
    const esc = {
      key: "Escape",
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    } as KeyboardEvent;
    expect(keyboardEventToHotkeyChord(esc)).toBe("escape");

    const j = {
      key: "j",
      ctrlKey: true,
      shiftKey: true,
      altKey: false,
      metaKey: false,
    } as KeyboardEvent;
    expect(keyboardEventToHotkeyChord(j)).toBe("ctrl+shift+j");

    const controlOnly = {
      key: "Control",
      ctrlKey: true,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    } as KeyboardEvent;
    expect(keyboardEventToHotkeyChord(controlOnly)).toBeNull();
  });

  it("waits for a main key after modifier-only presses during recording", () => {
    let state = createHotkeyRecordingState();
    const ctrlDown = {
      key: "Control",
      code: "ControlLeft",
      ctrlKey: true,
      shiftKey: false,
      altKey: false,
      metaKey: false,
      getModifierState: () => false,
    } as KeyboardEvent;
    let step = processHotkeyRecordingKeyDown(ctrlDown, state);
    state = step.next;
    expect(step.action).toEqual({ type: "ignore" });
    expect(state.heldModifiers.has("ctrl")).toBe(true);

    const spaceDown = {
      key: " ",
      code: "Space",
      ctrlKey: true,
      shiftKey: false,
      altKey: false,
      metaKey: false,
      getModifierState: (mod: string) => mod === "Control",
    } as KeyboardEvent;
    step = processHotkeyRecordingKeyDown(spaceDown, state);
    expect(step.action).toEqual({ type: "record", chord: "ctrl+space" });
  });

  it("records modifiers held with the main key on a single keydown", () => {
    const state = createHotkeyRecordingState();
    const j = {
      key: "j",
      code: "KeyJ",
      ctrlKey: true,
      shiftKey: true,
      altKey: false,
      metaKey: false,
      getModifierState: (mod: string) => mod === "Control" || mod === "Shift",
    } as KeyboardEvent;
    const step = processHotkeyRecordingKeyDown(j, state);
    expect(step.action).toEqual({ type: "record", chord: "ctrl+shift+j" });
  });

  it("drops released modifiers before the main key", () => {
    let state = createHotkeyRecordingState();
    const shiftDown = {
      key: "Shift",
      code: "ShiftLeft",
      ctrlKey: false,
      shiftKey: true,
      altKey: false,
      metaKey: false,
      getModifierState: (mod: string) => mod === "Shift",
    } as KeyboardEvent;
    state = processHotkeyRecordingKeyDown(shiftDown, state).next;

    const shiftUp = {
      key: "Shift",
      code: "ShiftLeft",
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
      getModifierState: () => false,
    } as KeyboardEvent;
    state = processHotkeyRecordingKeyUp(shiftUp, state);

    const spaceDown = {
      key: " ",
      code: "Space",
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
      getModifierState: () => false,
    } as KeyboardEvent;
    expect(processHotkeyRecordingKeyDown(spaceDown, state).action).toEqual({
      type: "record",
      chord: "space",
    });
  });

  it("ignores auto-repeat and IME composition during recording", () => {
    const state = createHotkeyRecordingState();
    const repeat = {
      key: "j",
      repeat: true,
      ctrlKey: true,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    } as KeyboardEvent;
    expect(isIgnorableHotkeyRecordingEvent(repeat)).toBe(true);
    expect(processHotkeyRecordingKeyDown(repeat, state).action).toEqual({ type: "ignore" });

    const composing = {
      key: "Process",
      isComposing: true,
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    } as KeyboardEvent;
    expect(processHotkeyRecordingKeyDown(composing, state).action).toEqual({ type: "ignore" });
  });

  it("cancels and clears recording on Escape and Backspace", () => {
    const state = createHotkeyRecordingState();
    const escape = {
      key: "Escape",
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    } as KeyboardEvent;
    expect(processHotkeyRecordingKeyDown(escape, state).action).toEqual({ type: "cancel" });

    const backspace = {
      key: "Backspace",
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    } as KeyboardEvent;
    expect(processHotkeyRecordingKeyDown(backspace, state).action).toEqual({ type: "clear" });
  });

  it("round-trips keyboard events through chord encoding and matching", () => {
    const samples = [
      {
        key: "Escape",
        ctrlKey: false,
        shiftKey: false,
        altKey: false,
        metaKey: false,
      },
      {
        key: "j",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      },
      {
        key: "F12",
        ctrlKey: true,
        shiftKey: false,
        altKey: false,
        metaKey: false,
      },
    ] as KeyboardEvent[];

    for (const event of samples) {
      const chord = keyboardEventToHotkeyChord(event);
      expect(chord).not.toBeNull();
      expect(hotkeyChordMatchesKeyboardEvent(chord!, event)).toBe(true);
    }
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

  it("auto-dismisses settings notices after 10 seconds", () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    const cancel = scheduleSettingsNoticeAutoDismiss(onDismiss);

    vi.advanceTimersByTime(SETTINGS_NOTICE_AUTO_DISMISS_MS - 1);
    expect(onDismiss).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(onDismiss).toHaveBeenCalledOnce();

    cancel();
    vi.useRealTimers();
  });

  it("fade-out tail runs after the configured fade duration", () => {
    vi.useFakeTimers();
    const onComplete = vi.fn();
    const cancel = scheduleSettingsNoticeFadeOut(onComplete);

    vi.advanceTimersByTime(SETTINGS_NOTICE_FADE_MS - 1);
    expect(onComplete).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(onComplete).toHaveBeenCalledOnce();

    cancel();
    vi.useRealTimers();
  });
});

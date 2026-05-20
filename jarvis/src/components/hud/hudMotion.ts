/**
 * HUD shell enter / exit timing — single source of truth for perceived fade length.
 *
 * Keep `HUD_WINDOW_HIDE_AFTER_FADE_MS` in `jarvis/src-tauri/src/lib.rs` aligned (~same ms + small buffer).
 */
export const HUD_SHELL_TRANSITION_MS = 320;

export const hudShellEase = [0.22, 1, 0.36, 1] as const;

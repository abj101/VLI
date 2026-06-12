import type { HudPhase } from "../../types";

/**
 * Phases that mount the glass HUD shell (`HudShell`).
 *
 * `matched` / `executing` are excluded: the overlay dismisses as soon as a command runs.
 * `done` is excluded: click-through + auto-dismiss hide the native window afterward.
 * `awaiting_input` stays so follow-up prompts can show the shell again.
 */
export const HUD_OVERLAY_SHELL_PHASES: readonly HudPhase[] = [
  "listening",
  "awaiting_input",
] as const;

export function isHudOverlayShellActive(phase: HudPhase): boolean {
  return (HUD_OVERLAY_SHELL_PHASES as readonly string[]).includes(phase);
}

import type { HudPhase } from "../../types";

/**
 * Phases that mount the glass HUD shell (`HudShell`).
 *
 * `done` is excluded: the backend still uses it for click-through + auto-dismiss,
 * but the React shell unmounts immediately so an empty frosted panel does not linger
 * until the window hides.
 */
export const HUD_OVERLAY_SHELL_PHASES: readonly HudPhase[] = [
  "listening",
  "matched",
  "executing",
  "awaiting_input",
] as const;

export function isHudOverlayShellActive(phase: HudPhase): boolean {
  return (HUD_OVERLAY_SHELL_PHASES as readonly string[]).includes(phase);
}

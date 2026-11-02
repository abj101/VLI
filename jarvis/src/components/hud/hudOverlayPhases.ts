import type { HudPhase } from "../../types";

/**
 * Phases that mount the glass HUD shell (`HudShell`).
 *
 * Keep execution and completion visible until the backend dismisses the session.
 * `stopped` starts the shell fade before the native window is hidden.
 */
export const HUD_OVERLAY_SHELL_PHASES: readonly HudPhase[] = [
  "listening",
  "routing",
  "matched",
  "executing",
  "done",
  "awaiting_input",
] as const;

export function isHudOverlayShellActive(phase: HudPhase): boolean {
  return (HUD_OVERLAY_SHELL_PHASES as readonly string[]).includes(phase);
}

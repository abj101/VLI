import type { MatchResult } from "../../types";
import type { HudPhase } from "../../types";

type CenterSelectorInput = {
  phase: HudPhase;
  transcript: string;
  match: MatchResult | null;
  actionText: string | null;
  actionError: string | null;
  audioError: string | null;
};

type CenterSelectorResult =
  | { kind: "error"; text: string }
  | { kind: "match" }
  | { kind: "action"; text: string }
  | { kind: "transcript"; text: string }
  | { kind: "placeholder" };

export function selectCenterContent(
  input: CenterSelectorInput,
): CenterSelectorResult {
  const normalizedActionText = normalizeActionText(input.actionText);

  if (
    (input.phase === "listening" || input.phase === "awaiting_input") &&
    input.audioError
  ) {
    return { kind: "error", text: input.audioError };
  }

  if (input.actionError) {
    return { kind: "error", text: input.actionError };
  }

  if (normalizedActionText === "follow up") {
    return { kind: "action", text: normalizedActionText };
  }

  if (input.match) {
    return { kind: "match" };
  }

  const transcript = input.transcript.trim();
  if (input.phase === "awaiting_input" && transcript.length > 0) {
    return { kind: "transcript", text: input.transcript };
  }

  if (
    normalizedActionText &&
    (input.phase === "matched" ||
      input.phase === "executing" ||
      input.phase === "awaiting_input" ||
      input.phase === "done")
  ) {
    return { kind: "action", text: normalizedActionText };
  }

  if (transcript.length > 0) {
    return { kind: "transcript", text: input.transcript };
  }

  return { kind: "placeholder" };
}

function normalizeActionText(actionText: string | null): string | null {
  if (!actionText) return null;
  const text = actionText.trim();
  const lowered = text.toLowerCase();
  if (
    lowered === "follow up" ||
    lowered.startsWith("awaiting input:") ||
    lowered.startsWith("awaiting follow-up:")
  ) {
    return "follow up";
  }
  return text;
}

export function selectPhaseLabel(phase: HudPhase): string | null {
  void phase;
  return null;
}

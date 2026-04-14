import type { CommandNodePayload } from "../../types";

export function getPrimaryTriggerPhrase(node: CommandNodePayload): string {
  return node.trigger_phrases[0] ?? "(no trigger phrase)";
}

export function withEnabledValue(
  node: CommandNodePayload,
  enabled: boolean,
): CommandNodePayload {
  return { ...node, enabled };
}

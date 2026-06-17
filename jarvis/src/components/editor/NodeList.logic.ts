import type { CommandNodePayload } from "../../types";

export function withEnabledValue(
  node: CommandNodePayload,
  enabled: boolean,
): CommandNodePayload {
  return { ...node, enabled };
}

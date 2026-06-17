import { describe, expect, it } from "vitest";
import type { CommandNodePayload } from "../../types";
import { withEnabledValue } from "./NodeList.logic";

function makeNode(enabled = true): CommandNodePayload {
  return {
    id: 1,
    name: "test",
    trigger_phrases: ["open calculator"],
    actions: [{ wait: { ms: 10 } }],
    enabled,
    fuzzy_threshold_pct: 75,
    created_at: "2026-01-01T00:00:00Z",
  };
}

describe("NodeList logic helpers", () => {
  it("withEnabledValue sets enabled flag", () => {
    const node = makeNode(true);
    expect(withEnabledValue(node, false).enabled).toBe(false);
    expect(withEnabledValue(node, true).enabled).toBe(true);
  });
});

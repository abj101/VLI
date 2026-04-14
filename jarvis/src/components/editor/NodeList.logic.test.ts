import { describe, expect, it } from "vitest";
import type { CommandNodePayload } from "../../types";
import { getPrimaryTriggerPhrase, withEnabledValue } from "./NodeList.logic";

function makeNode(): CommandNodePayload {
  return {
    id: 9,
    name: "open calc",
    trigger_phrases: ["open calculator", "start calc"],
    actions: [{ open_app: { name: "Calculator", path: "calc.exe" } }],
    enabled: true,
    fuzzy_threshold_pct: 75,
    created_at: "2026-01-01T00:00:00Z",
  };
}

describe("NodeList logic helpers", () => {
  it("returns first trigger phrase when present", () => {
    expect(getPrimaryTriggerPhrase(makeNode())).toBe("open calculator");
  });

  it("returns fallback text when trigger phrase list is empty", () => {
    const node = { ...makeNode(), trigger_phrases: [] };
    expect(getPrimaryTriggerPhrase(node)).toBe("(no trigger phrase)");
  });

  it("returns cloned node with changed enabled value", () => {
    const node = makeNode();
    const next = withEnabledValue(node, false);

    expect(next.enabled).toBe(false);
    expect(next.id).toBe(node.id);
    expect(next).not.toBe(node);
  });
});

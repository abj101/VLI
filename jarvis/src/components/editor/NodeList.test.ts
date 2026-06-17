import { beforeEach, describe, expect, it } from "vitest";
import { useEditorStore } from "../../store/editorStore";
import type { CommandNodePayload } from "../../types";

function makeNode(id: number, enabled = true): CommandNodePayload {
  return {
    id,
    name: `node-${id}`,
    trigger_phrases: [`trigger-${id}`],
    actions: [{ wait: { ms: 10 } }],
    enabled,
    fuzzy_threshold_pct: 75,
    created_at: "2026-01-01T00:00:00Z",
  };
}

describe("editorStore", () => {
  beforeEach(() => {
    useEditorStore.setState({ nodes: [] });
  });

  it("setNodes replaces nodes list", () => {
    const nodes = [makeNode(1), makeNode(2)];
    useEditorStore.getState().setNodes(nodes);

    expect(useEditorStore.getState().nodes).toEqual(nodes);
  });

  it("deleteNode removes matching node", () => {
    useEditorStore.setState({
      nodes: [makeNode(1), makeNode(2), makeNode(3)],
    });

    useEditorStore.getState().deleteNode(2);

    expect(useEditorStore.getState().nodes.map((n) => n.id)).toEqual([1, 3]);
  });

  it("toggleEnabled flips enabled value for matching node", () => {
    useEditorStore.setState({
      nodes: [makeNode(1, true), makeNode(2, false)],
    });

    useEditorStore.getState().toggleEnabled(1);
    useEditorStore.getState().toggleEnabled(2);

    const [first, second] = useEditorStore.getState().nodes;
    expect(first.enabled).toBe(false);
    expect(second.enabled).toBe(true);
  });
});

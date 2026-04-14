import { describe, expect, it } from "vitest";
import {
  makeRowId,
  moveByArrow,
  parseRowId,
  reorderActionsFromDrag,
  reorderByMove,
} from "./ActionChain.logic";

describe("ActionChain logic", () => {
  it("makeRowId and parseRowId round-trip", () => {
    expect(makeRowId(0)).toBe("row-0");
    expect(parseRowId("row-3")).toBe(3);
    expect(parseRowId("nope")).toBe(-1);
  });

  it("reorderByMove moves item to new index", () => {
    expect(reorderByMove([1, 2, 3], 2, 0)).toEqual([3, 1, 2]);
    expect(reorderByMove([1, 2, 3], 1, 1)).toEqual([1, 2, 3]);
  });

  it("moveByArrow swaps with neighbor", () => {
    expect(moveByArrow(["a", "b", "c"], 1, -1)).toEqual(["b", "a", "c"]);
    expect(moveByArrow(["a", "b", "c"], 0, -1)).toEqual(["a", "b", "c"]);
    expect(moveByArrow(["a", "b", "c"], 2, 1)).toEqual(["a", "b", "c"]);
  });

  it("reorderActionsFromDrag returns null on invalid or no-op", () => {
    const actions = [{ wait: { ms: 1 } }, { wait: { ms: 2 } }];
    expect(reorderActionsFromDrag(actions, "row-0", "row-0")).toBeNull();
    expect(reorderActionsFromDrag(actions, "bad", "row-1")).toBeNull();
  });

  it("reorderActionsFromDrag reorders actions", () => {
    const actions = [{ wait: { ms: 1 } }, { wait: { ms: 2 } }, { wait: { ms: 3 } }];
    const next = reorderActionsFromDrag(actions, "row-2", "row-0");
    expect(next).toEqual([{ wait: { ms: 3 } }, { wait: { ms: 1 } }, { wait: { ms: 2 } }]);
  });
});

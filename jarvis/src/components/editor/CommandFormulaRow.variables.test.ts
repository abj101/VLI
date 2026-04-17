import { describe, expect, it } from "vitest";
import type { FormActionPayload } from "../../types";
import { deriveFollowUpVariableMap, extractVariableTokenContext } from "./CommandFormulaRow";

describe("CommandFormulaRow variable helpers", () => {
  it("numbers follow-up variables in action order for one row", () => {
    const actions: FormActionPayload[] = [
      { sub_prompt: { prompt: "first?" } },
      { open_url: { url: "https://example.com" } },
      { sub_prompt: { prompt: "second?" } },
    ];
    const meta = deriveFollowUpVariableMap(actions);
    expect(meta.labels).toEqual(["Variable 1", "Variable 2"]);
    expect(meta.byActionIndex.get(0)).toBe(1);
    expect(meta.byActionIndex.get(2)).toBe(2);
    expect(meta.byActionIndex.get(1)).toBeUndefined();
  });

  it("extracts Variable token context at caret", () => {
    const value = "Open Variable 2";
    const token = extractVariableTokenContext(value, value.length);
    expect(token).toEqual({
      start: 5,
      end: value.length,
      query: "2",
    });
  });

  it("ignores unrelated input", () => {
    const value = "Action";
    expect(extractVariableTokenContext(value, value.length)).toBeNull();
  });
});

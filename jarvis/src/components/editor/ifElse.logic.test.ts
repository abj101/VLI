import { describe, expect, it } from "vitest";
import {
  defaultIfElseAction,
  ifElseConditionNeedsPattern,
  validateIfElseFields,
} from "./ifElse.logic";

describe("ifElse.logic", () => {
  it("defaultIfElseAction starts with text_contains and empty branches", () => {
    expect(defaultIfElseAction()).toEqual({
      if_else: {
        condition: "text_contains",
        text: "",
        pattern: "",
        then_actions: [],
        else_actions: [],
      },
    });
  });

  it("ifElseConditionNeedsPattern is false only for is_empty", () => {
    expect(ifElseConditionNeedsPattern("text_is_empty")).toBe(false);
    expect(ifElseConditionNeedsPattern("text_contains")).toBe(true);
    expect(ifElseConditionNeedsPattern("regex_match")).toBe(true);
  });

  it("validateIfElseFields requires pattern for contains/regex", () => {
    const base = defaultIfElseAction().if_else;
    expect(validateIfElseFields({ ...base, then_actions: [{ speak: { text: "hi" } }] }).pattern).toBe(
      "Pattern is required for this condition.",
    );
    expect(
      validateIfElseFields({
        ...base,
        pattern: "foo",
        then_actions: [{ speak: { text: "hi" } }],
      }).pattern,
    ).toBeUndefined();
  });

  it("validateIfElseFields requires at least one branch action", () => {
    expect(validateIfElseFields(defaultIfElseAction().if_else).then).toBe(
      "Add at least one Then or Else action.",
    );
  });
});

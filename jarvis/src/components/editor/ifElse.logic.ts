import type { ActionPayload } from "../../types";

export type IfConditionKind = "text_contains" | "regex_match" | "text_is_empty";

export type IfElseAction = {
  condition: IfConditionKind;
  text: string;
  pattern: string;
  then_actions: ActionPayload[];
  else_actions: ActionPayload[];
};

export function defaultIfElseAction(): { if_else: IfElseAction } {
  return {
    if_else: {
      condition: "text_contains",
      text: "",
      pattern: "",
      then_actions: [],
      else_actions: [],
    },
  };
}

export function ifElseConditionNeedsPattern(condition: IfConditionKind): boolean {
  return condition !== "text_is_empty";
}

export function ifElseConditionLabel(condition: IfConditionKind): string {
  switch (condition) {
    case "text_contains":
      return "contains";
    case "regex_match":
      return "matches regex";
    case "text_is_empty":
      return "is empty";
  }
}

export function validateIfElseFields(
  action: IfElseAction,
): { text?: string; pattern?: string; then?: string; else?: string } {
  const errors: { text?: string; pattern?: string; then?: string; else?: string } = {};
  if (ifElseConditionNeedsPattern(action.condition) && action.pattern.trim().length === 0) {
    errors.pattern = "Pattern is required for this condition.";
  }
  if (action.then_actions.length === 0 && action.else_actions.length === 0) {
    errors.then = "Add at least one Then or Else action.";
  }
  return errors;
}

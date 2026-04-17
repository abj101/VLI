import { isEditorPendingAction, type FormActionPayload } from "../../types";
import type { ActionKind, ConcreteActionKind } from "./NodeForm.logic";

export type ActionKindOption = {
  id: ConcreteActionKind;
  label: string;
  /** Extra tokens for search (lowercase). */
  haystack: string;
};

export const ACTION_KIND_OPTIONS: ActionKindOption[] = [
  { id: "open_app", label: "Open app", haystack: "open application launch program exe" },
  { id: "open_url", label: "Open URL", haystack: "open url link web http https browser" },
  { id: "send_keys", label: "Send keys", haystack: "send keys keyboard shortcut hotkey type" },
  { id: "speak", label: "Speak", haystack: "speak say voice tts read aloud" },
  { id: "wait", label: "Wait", haystack: "wait pause delay ms milliseconds" },
  {
    id: "sub_prompt",
    label: "Follow Up",
    haystack: "follow up sub prompt question ask input voice",
  },
];

export function getActionKind(action: FormActionPayload): ActionKind {
  if (isEditorPendingAction(action)) return "pending";
  if ("open_app" in action) return "open_app";
  if ("open_url" in action) return "open_url";
  if ("run_script" in action) return "run_script";
  if ("send_keys" in action) return "send_keys";
  if ("speak" in action) return "speak";
  if ("wait" in action) return "wait";
  return "sub_prompt";
}

export function actionKindLabel(kind: ActionKind): string {
  if (kind === "pending") return "";
  return ACTION_KIND_OPTIONS.find((o) => o.id === kind)?.label ?? kind;
}

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
  {
    id: "open_target",
    label: "Open target",
    haystack: "open target app site url smart remainder placement",
  },
  {
    id: "place_window",
    label: "Snap window",
    haystack: "snap window place left right maximize zone",
  },
  { id: "send_keys", label: "Send keys", haystack: "send keys keyboard shortcut hotkey type" },
  { id: "speak", label: "Speak", haystack: "speak say voice tts read aloud" },
  { id: "wait", label: "Wait", haystack: "wait pause delay ms milliseconds" },
  {
    id: "sub_prompt",
    label: "Follow Up",
    haystack: "follow up sub prompt question ask input voice",
  },
  {
    id: "run_command",
    label: "Run command",
    haystack: "run command compose nested shortcut pipeline",
  },
  {
    id: "get_clipboard",
    label: "Get clipboard",
    haystack: "get clipboard copy paste read text",
  },
  {
    id: "http_get",
    label: "HTTP GET",
    haystack: "http get fetch web request api download url",
  },
  {
    id: "read_file",
    label: "Read file",
    haystack: "read file text contents path open document",
  },
  {
    id: "show_notification",
    label: "Show notification",
    haystack: "show notification toast alert message banner",
  },
  {
    id: "text_trim",
    label: "Trim text",
    haystack: "trim text whitespace strip space",
  },
  {
    id: "text_match",
    label: "Match text",
    haystack: "match text regex pattern find extract",
  },
  {
    id: "text_split",
    label: "Split text",
    haystack: "split text delimiter separate list",
  },
  {
    id: "text_combine",
    label: "Combine text",
    haystack: "combine text join merge list separator",
  },
  {
    id: "set_clipboard",
    label: "Set clipboard",
    haystack: "set clipboard copy paste write text",
  },
  {
    id: "list_folder",
    label: "List folder",
    haystack: "list folder directory entries files browse",
  },
  {
    id: "write_file",
    label: "Write file",
    haystack: "write file save text document path",
  },
  {
    id: "get_file_metadata",
    label: "File metadata",
    haystack: "get file metadata size modified directory stat",
  },
  {
    id: "screenshot",
    label: "Screenshot",
    haystack: "screenshot capture screen display image png",
  },
  {
    id: "device_info",
    label: "Device info",
    haystack: "device info cpu memory ram system stats",
  },
  {
    id: "start_dictation",
    label: "Start dictation",
    haystack: "start dictation voice type speak microphone live text field",
  },
  {
    id: "stop_dictation",
    label: "Stop dictation",
    haystack: "stop dictation voice type microphone end",
  },
  {
    id: "if_else",
    label: "If/Else",
    haystack: "if else condition branch logic contains regex empty",
  },
];

export function getActionKind(action: FormActionPayload): ActionKind {
  if (isEditorPendingAction(action)) return "pending";
  if ("open_app" in action) return "open_app";
  if ("open_url" in action) return "open_url";
  if ("open_target" in action) return "open_target";
  if ("place_window" in action) return "place_window";
  if ("run_script" in action) return "run_script";
  if ("send_keys" in action) return "send_keys";
  if ("speak" in action) return "speak";
  if ("wait" in action) return "wait";
  if ("run_command" in action) return "run_command";
  if ("read_file" in action) return "read_file";
  if ("http_get" in action) return "http_get";
  if ("get_clipboard" in action) return "get_clipboard";
  if ("show_notification" in action) return "show_notification";
  if ("text_trim" in action) return "text_trim";
  if ("text_match" in action) return "text_match";
  if ("text_split" in action) return "text_split";
  if ("text_combine" in action) return "text_combine";
  if ("set_clipboard" in action) return "set_clipboard";
  if ("list_folder" in action) return "list_folder";
  if ("write_file" in action) return "write_file";
  if ("get_file_metadata" in action) return "get_file_metadata";
  if ("screenshot" in action) return "screenshot";
  if ("device_info" in action) return "device_info";
  if ("start_dictation" in action) return "start_dictation";
  if ("stop_dictation" in action) return "stop_dictation";
  if ("if_else" in action) return "if_else";
  return "sub_prompt";
}

export function actionKindLabel(kind: ActionKind): string {
  if (kind === "pending") return "";
  return ACTION_KIND_OPTIONS.find((o) => o.id === kind)?.label ?? kind;
}

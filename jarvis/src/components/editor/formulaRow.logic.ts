import type { ActionPayload, CommandNodePayload } from "../../types";
import { getActionKind } from "./actionCatalog";

export function fingerprintCommandNode(node: CommandNodePayload): string {
  return JSON.stringify({
    id: node.id,
    name: node.name,
    trigger_phrases: node.trigger_phrases,
    actions: node.actions,
    enabled: node.enabled,
    fuzzy_threshold_pct: node.fuzzy_threshold_pct,
  });
}

function actionSnippet(action: ActionPayload): string {
  if ("open_app" in action) return action.open_app.name || action.open_app.path;
  if ("open_url" in action) return action.open_url.url;
  if ("run_script" in action) return action.run_script.script;
  if ("send_keys" in action) return action.send_keys.keys;
  if ("speak" in action) return action.speak.text;
  if ("wait" in action) return String(action.wait.ms);
  return action.sub_prompt.prompt;
}

export function commandNodeSearchHaystack(node: CommandNodePayload): string {
  const bits = [
    node.name,
    ...node.trigger_phrases,
    ...node.actions.map((a) => `${getActionKind(a)} ${actionSnippet(a)}`),
  ];
  return bits.join(" ").toLowerCase();
}

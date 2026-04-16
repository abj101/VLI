import type { ActionPayload, CommandNodePayload } from "../../types";
import { getActionKind } from "./actionCatalog";

export function fingerprintCommandNode(node: CommandNodePayload): string {
  return JSON.stringify({
    id: node.id,
    trigger_phrases: node.trigger_phrases,
    actions: node.actions,
    enabled: node.enabled,
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
    ...node.trigger_phrases,
    ...node.actions.map((a) => `${getActionKind(a)} ${actionSnippet(a)}`),
  ];
  return bits.join(" ").toLowerCase();
}

type DeriveAppSearchMetaInput = {
  isOpen: boolean;
  query: string;
  isLoading: boolean;
  hasSearched: boolean;
  hitCount: number;
  /** Known installed-app index size. `null` = not yet reported, `0` = empty. */
  indexCount?: number | null;
  /** Is a background scan currently running? */
  isScanning?: boolean;
};

type AppSearchMeta = {
  statusText: string | null;
  countText: string | null;
};

export function deriveAppSearchMeta(input: DeriveAppSearchMetaInput): AppSearchMeta {
  if (!input.isOpen) {
    return { statusText: null, countText: null };
  }
  if (input.isLoading) {
    return { statusText: "Searching…", countText: null };
  }
  // Tell the user we're scanning **only** while a scan is actually in flight
  // or the index has never reported (`null`). Once a scan has finished with
  // a result — even zero entries — we fall through to the usual no-match
  // messaging so the dropdown isn't stuck on "Indexing apps…" forever.
  const countKnown = input.indexCount !== undefined && input.indexCount !== null;
  if (input.isScanning || (!countKnown && input.hitCount === 0)) {
    return { statusText: "Indexing apps…", countText: null };
  }
  const hasQuery = input.query.trim().length > 0;
  if (!hasQuery || !input.hasSearched) {
    return { statusText: null, countText: null };
  }
  if (input.hitCount === 0) {
    return {
      statusText: `No apps match "${input.query.trim()}"`,
      countText: null,
    };
  }
  return {
    statusText: null,
    countText: `Found ${input.hitCount} app${input.hitCount === 1 ? "" : "s"}`,
  };
}

type DeriveOpenAppDisplayModeInput = {
  isEditing: boolean;
  selectedPath: string;
};

export function deriveOpenAppDisplayMode(input: DeriveOpenAppDisplayModeInput): "edit" | "confirmed" {
  if (input.isEditing) {
    return "edit";
  }
  if (input.selectedPath.trim().length === 0) {
    return "edit";
  }
  return "confirmed";
}

type FormulaArgInputClassOptions = {
  narrow?: boolean;
  autoGrow?: boolean;
};

export function formulaArgInputClass(options: FormulaArgInputClassOptions = {}): string {
  const classes = ["editor-formula-input", "editor-formula-input--arg"];
  if (options.narrow) {
    classes.push("editor-formula-input--narrow");
  }
  if (options.autoGrow ?? true) {
    classes.push("editor-formula-input--autogrow");
  }
  return classes.join(" ");
}

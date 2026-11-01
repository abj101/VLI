import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { useEditorStore } from "../../store/editorStore";
import type { CommandNodePayload } from "../../types";
import { formatUserError } from "../../utils/userErrors";
import { actionKindLabel, getActionKind } from "./actionCatalog";
import { commandNodeSearchHaystack } from "./formulaRow.logic";
import { CommandDeleteConfirm } from "./CommandDeleteConfirm";

export function CommandsTab() {
  const nodes = useEditorStore((s) => s.nodes);
  const setNodes = useEditorStore((s) => s.setNodes);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [deletePending, setDeletePending] = useState<number | null>(null);

  useEffect(() => {
    let mounted = true;
    const load = async () => {
      try {
        const list = await invoke<CommandNodePayload[]>("list_commands");
        if (mounted) {
          setNodes(list);
          setError(null);
        }
      } catch (err) {
        if (mounted) setError(formatUserError(err, "Could not load commands."));
      }
    };
    void load();
    let unlisten: (() => void) | undefined;
    void listen("editor-commands-changed", () => void load()).then((off) => {
      if (mounted) unlisten = off;
      else off();
    });
    return () => { mounted = false; unlisten?.(); };
  }, [setNodes]);

  const search = query.trim().toLowerCase();
  const filtered = nodes.filter((node) =>
    `${node.name.toLowerCase()} ${commandNodeSearchHaystack(node)}`.includes(search),
  );

  const deleteCommand = async (id: number) => {
    try {
      if (await invoke<boolean>("delete_command", { id })) {
        useEditorStore.getState().deleteNode(id);
        setDeletePending(null);
      } else {
        setError("Could not delete command.");
      }
    } catch (err) {
      setError(formatUserError(err, "Could not delete command."));
    }
  };

  return (
    <div className="editor-commands-tab">
      <header className="editor-commands-toolbar">
        <input className="editor-formula-input editor-commands-search" value={query}
          onChange={(e) => setQuery(e.target.value)} placeholder="Search" aria-label="Search commands" />
      </header>
      {error && <div className="editor-inline-toast" role="alert">{error}</div>}
      {filtered.length === 0 ? (
        <div className="editor-commands-empty" role="status">
          {nodes.length === 0 ? "No commands yet." : "No commands match your search."}
        </div>
      ) : (
        <ul className="editor-command-list">
          {filtered.map((node) => (
            <li className="editor-command-item editor-command-readonly" key={node.id}>
              <div>
                <strong>{node.trigger_phrases.join(", ") || node.name}</strong>
                <div className="editor-command-readonly-actions">
                  {node.actions.map((action) => actionKindLabel(getActionKind(action))).join(" → ") || "No actions"}
                </div>
              </div>
              <div className="editor-command-readonly-controls">
                {!node.enabled && <span className="editor-command-readonly-status">Disabled</span>}
                <CommandDeleteConfirm
                  deletePending={deletePending === node.id}
                  phraseLabel={node.trigger_phrases[0] || node.name}
                  onRequestDelete={() => setDeletePending(node.id)}
                  onCancel={() => setDeletePending(null)}
                  onConfirm={() => void deleteCommand(node.id)}
                />
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

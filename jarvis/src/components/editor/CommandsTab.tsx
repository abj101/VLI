import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useRef, useState } from "react";
import { formatUserError } from "../../utils/userErrors";
import { useEditorStore } from "../../store/editorStore";
import type { CommandNodePayload } from "../../types";
import { CommandDraftRow, CommandFormulaRow } from "./CommandFormulaRow";
import { commandNodeSearchHaystack } from "./formulaRow.logic";
import { getPrimaryTriggerPhrase, withEnabledValue } from "./NodeList.logic";

export function CommandsTab() {
  const nodes = useEditorStore((s) => s.nodes);
  const setNodes = useEditorStore((s) => s.setNodes);
  const deleteNode = useEditorStore((s) => s.deleteNode);
  const toggleEnabled = useEditorStore((s) => s.toggleEnabled);

  const [errorText, setErrorText] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [showDraft, setShowDraft] = useState(false);
  const errorTimeoutRef = useRef<number | null>(null);

  const showError = (text: string) => {
    setErrorText(text);
    if (errorTimeoutRef.current) window.clearTimeout(errorTimeoutRef.current);
    errorTimeoutRef.current = window.setTimeout(() => {
      setErrorText(null);
      errorTimeoutRef.current = null;
    }, 2200);
  };

  useEffect(() => {
    let mounted = true;
    const loadNodes = async () => {
      try {
        const list = await invoke<CommandNodePayload[]>("list_commands");
        if (!mounted) return;
        setNodes(list);
      } catch (err: unknown) {
        if (!mounted) return;
        showError(formatUserError(err, "Could not load commands."));
      }
    };
    void loadNodes();

    let unlisten: (() => void) | null = null;
    void listen("editor-commands-changed", () => {
      void loadNodes();
    }).then((off) => {
      if (mounted) unlisten = off;
      else off();
    });

    return () => {
      mounted = false;
      unlisten?.();
      if (errorTimeoutRef.current) window.clearTimeout(errorTimeoutRef.current);
    };
  }, [setNodes]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return nodes;
    return nodes.filter((n) => commandNodeSearchHaystack(n).includes(q));
  }, [nodes, query]);

  const onToggleEnabled = (id: number) => {
    const current = useEditorStore.getState().nodes.find((n) => n.id === id);
    if (!current) return;
    const nextEnabled = !current.enabled;
    toggleEnabled(id);
    void invoke<CommandNodePayload>("update_command", {
      id,
      node: withEnabledValue(current, nextEnabled),
    })
      .then((saved) => {
        const latest = useEditorStore.getState().nodes;
        setNodes(latest.map((n) => (n.id === saved.id ? saved : n)));
      })
      .catch((err: unknown) => {
        toggleEnabled(id);
        showError(formatUserError(err, "Could not update that command."));
      });
  };

  const onDelete = (id: number) => {
    const node = useEditorStore.getState().nodes.find((e) => e.id === id);
    if (!node) return;
    if (!window.confirm(`Delete "${getPrimaryTriggerPhrase(node)}"?`)) return;
    void invoke<boolean>("delete_command", { id })
      .then((deleted) => {
        if (deleted) {
          deleteNode(id);
          return;
        }
        showError("Delete failed.");
      })
      .catch((err: unknown) => {
        showError(formatUserError(err, "Could not delete that command."));
      });
  };

  return (
    <div className="editor-commands-tab">
      <header className="editor-commands-toolbar">
        <input
          type="search"
          className="editor-formula-input editor-commands-search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search"
          aria-label="Search commands"
        />
        <button
          type="button"
          className="editor-commands-add"
          onClick={() => setShowDraft(true)}
          aria-label="Add command"
        >
          +
        </button>
      </header>

      {errorText && (
        <div className="editor-inline-toast" role="alert">
          {errorText}
        </div>
      )}

      {nodes.length === 0 && !showDraft ? (
        <div className="editor-empty-state editor-commands-empty">
          <p>No commands yet.</p>
          <button
            type="button"
            className="editor-empty-add"
            onClick={() => setShowDraft(true)}
            aria-label="Add command"
          >
            +
          </button>
        </div>
      ) : (
        <>
          <ul className="editor-command-list">
            {showDraft && (
              <CommandDraftRow
                onDiscard={() => setShowDraft(false)}
                onCreated={async () => {
                  try {
                    const list = await invoke<CommandNodePayload[]>("list_commands");
                    setNodes(list);
                  } catch {
                    showError("Created, but list refresh failed.");
                  }
                  setShowDraft(false);
                }}
              />
            )}
            {filtered.map((node) => (
              <CommandFormulaRow
                key={node.id}
                node={node}
                onToggleEnabled={() => onToggleEnabled(node.id)}
                onDelete={() => onDelete(node.id)}
              />
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

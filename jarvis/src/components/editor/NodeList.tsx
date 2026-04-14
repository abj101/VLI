import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { useEditorStore } from "../../store/editorStore";
import type { CommandNodePayload } from "../../types";
import { getPrimaryTriggerPhrase, withEnabledValue } from "./NodeList.logic";

export function NodeList() {
  const nodes = useEditorStore((s) => s.nodes);
  const selectedId = useEditorStore((s) => s.selectedId);
  const setSelected = useEditorStore((s) => s.setSelected);
  const setNodes = useEditorStore((s) => s.setNodes);
  const deleteNode = useEditorStore((s) => s.deleteNode);
  const toggleEnabled = useEditorStore((s) => s.toggleEnabled);

  const [errorText, setErrorText] = useState<string | null>(null);
  const errorTimeoutRef = useRef<number | null>(null);

  const showError = (text: string) => {
    setErrorText(text);
    if (errorTimeoutRef.current) {
      window.clearTimeout(errorTimeoutRef.current);
    }
    errorTimeoutRef.current = window.setTimeout(() => {
      setErrorText(null);
      errorTimeoutRef.current = null;
    }, 2000);
  };

  useEffect(() => {
    let mounted = true;
    void invoke<CommandNodePayload[]>("list_commands")
      .then((list) => {
        if (!mounted) return;
        setNodes(list);
      })
      .catch((err: unknown) => {
        if (!mounted) return;
        showError(`Failed to load commands: ${String(err)}`);
      });

    return () => {
      mounted = false;
      if (errorTimeoutRef.current) {
        window.clearTimeout(errorTimeoutRef.current);
      }
    };
  }, [setNodes]);

  const onToggle = (id: number) => {
    const current = useEditorStore.getState().nodes.find((node) => node.id === id);
    if (!current) return;

    const nextEnabled = !current.enabled;
    toggleEnabled(id);

    void invoke<CommandNodePayload>("update_command", {
      id,
      node: withEnabledValue(current, nextEnabled),
    })
      .then((saved) => {
        const latest = useEditorStore.getState().nodes;
        setNodes(latest.map((node) => (node.id === saved.id ? saved : node)));
      })
      .catch((err: unknown) => {
        toggleEnabled(id);
        showError(`Failed to update command: ${String(err)}`);
      });
  };

  const onDelete = (id: number) => {
    const node = useEditorStore.getState().nodes.find((entry) => entry.id === id);
    if (!node) return;

    if (!window.confirm(`Delete "${node.name}"?`)) return;

    void invoke<boolean>("delete_command", { id })
      .then((deleted) => {
        if (deleted) {
          deleteNode(id);
          return;
        }
        showError("Delete failed: command was not removed");
      })
      .catch((err: unknown) => {
        showError(`Delete failed: ${String(err)}`);
      });
  };

  return (
    <section className="editor-panel editor-panel-left">
      <header className="editor-panel-header">
        <h2>Nodes</h2>
        <button
          type="button"
          className="editor-add-btn"
          onClick={() => setSelected(null)}
          aria-label="Create new node"
        >
          +
        </button>
      </header>

      {errorText && (
        <div className="editor-inline-toast" role="alert">
          {errorText}
        </div>
      )}

      {nodes.length === 0 ? (
        <div className="editor-empty-state">
          <p>No command nodes yet.</p>
          <button
            type="button"
            className="editor-empty-add"
            onClick={() => setSelected(null)}
            aria-label="Create first command node"
          >
            +
          </button>
        </div>
      ) : (
        <ul className="editor-node-list">
          {nodes.map((node) => {
            const selected = node.id === selectedId;
            return (
              <li key={node.id}>
                <div
                  className={`editor-node-row${selected ? " is-selected" : ""}`}
                  onClick={() => setSelected(node.id)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      setSelected(node.id);
                    }
                  }}
                  role="button"
                  tabIndex={0}
                >
                  <span className="editor-node-main">
                    <span className="editor-node-name">{node.name}</span>
                    <span className="editor-node-trigger">
                      {getPrimaryTriggerPhrase(node)}
                    </span>
                  </span>

                  <span className="editor-node-actions">
                    <button
                      type="button"
                      className={`editor-toggle${node.enabled ? " is-on" : ""}`}
                      aria-pressed={node.enabled}
                      aria-label={node.enabled ? "Disable node" : "Enable node"}
                      onClick={(e) => {
                        e.stopPropagation();
                        onToggle(node.id);
                      }}
                    >
                      {node.enabled ? "On" : "Off"}
                    </button>
                    <button
                      type="button"
                      className="editor-delete-btn"
                      aria-label={`Delete ${node.name}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        onDelete(node.id);
                      }}
                    >
                      Delete
                    </button>
                  </span>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

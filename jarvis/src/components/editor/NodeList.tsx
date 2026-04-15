import {
  DndContext,
  PointerSensor,
  closestCenter,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import { CSS } from "@dnd-kit/utilities";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { formatUserError } from "../../utils/userErrors";
import { useEditorStore } from "../../store/editorStore";
import type { CommandNodePayload } from "../../types";
import {
  getPrimaryTriggerPhrase,
  makeNodeRowId,
  parseNodeRowId,
  reorderIdsByArrow,
  reorderIdsByDrag,
  withEnabledValue,
} from "./NodeList.logic";

export function NodeList() {
  const nodes = useEditorStore((s) => s.nodes);
  const selectedId = useEditorStore((s) => s.selectedId);
  const setSelected = useEditorStore((s) => s.setSelected);
  const setNodes = useEditorStore((s) => s.setNodes);
  const reorderNodes = useEditorStore((s) => s.reorderNodes);
  const deleteNode = useEditorStore((s) => s.deleteNode);
  const toggleEnabled = useEditorStore((s) => s.toggleEnabled);
  const sensors = useSensors(useSensor(PointerSensor));

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
    const loadNodes = async () => {
      try {
        const list = await invoke<CommandNodePayload[]>("list_commands");
        if (!mounted) return;
        setNodes(list);
      } catch (err: unknown) {
        if (!mounted) return;
        showError(formatUserError(err, "Could not load commands. Try reopening the editor."));
      }
    };
    void loadNodes();

    let unlisten: (() => void) | null = null;
    void listen("editor-commands-changed", () => {
      void loadNodes();
    }).then((off) => {
      if (mounted) {
        unlisten = off;
      } else {
        off();
      }
    });

    return () => {
      mounted = false;
      if (unlisten) {
        unlisten();
      }
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
        showError(formatUserError(err, "Could not update that command."));
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
        showError(formatUserError(err, "Could not delete that command."));
      });
  };

  const persistReorder = (orderedIds: number[]) => {
    const previousNodes = useEditorStore.getState().nodes;
    reorderNodes(orderedIds);
    void invoke("reorder_commands", { payload: { orderedIds } }).catch((err: unknown) => {
      setNodes(previousNodes);
      showError(formatUserError(err, "Could not reorder commands."));
    });
  };

  const onDragEnd = (event: DragEndEvent) => {
    if (!event.over) return;
    const activeId = parseNodeRowId(String(event.active.id));
    const overId = parseNodeRowId(String(event.over.id));
    const currentIds = useEditorStore.getState().nodes.map((node) => node.id);
    const reordered = reorderIdsByDrag(currentIds, activeId, overId);
    if (reordered === currentIds) return;
    persistReorder(reordered);
  };

  const onArrowReorder = (id: number, direction: -1 | 1) => {
    const currentIds = useEditorStore.getState().nodes.map((node) => node.id);
    const reordered = reorderIdsByArrow(currentIds, id, direction);
    if (reordered === currentIds) return;
    persistReorder(reordered);
  };

  return (
    <section className="editor-panel editor-glass-panel editor-panel-left">
      <header className="editor-panel-header">
        <h2>Commands</h2>
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
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
          <ul className="editor-node-list">
            {nodes.map((node) => (
              <NodeRow
                key={node.id}
                node={node}
                selected={node.id === selectedId}
                onSelect={() => setSelected(node.id)}
                onToggle={() => onToggle(node.id)}
                onDelete={() => onDelete(node.id)}
                onMoveUp={() => onArrowReorder(node.id, -1)}
                onMoveDown={() => onArrowReorder(node.id, 1)}
              />
            ))}
          </ul>
        </DndContext>
      )}
    </section>
  );
}

type NodeRowProps = {
  node: CommandNodePayload;
  selected: boolean;
  onSelect: () => void;
  onToggle: () => void;
  onDelete: () => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
};

function NodeRow({
  node,
  selected,
  onSelect,
  onToggle,
  onDelete,
  onMoveUp,
  onMoveDown,
}: NodeRowProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuWrapRef = useRef<HTMLDivElement>(null);
  const rowId = makeNodeRowId(node.id);
  const { setNodeRef: setDropRef } = useDroppable({ id: rowId });
  const { attributes, listeners, setNodeRef: setDragRef, transform } = useDraggable({
    id: rowId,
  });
  const style = { transform: CSS.Translate.toString(transform) };

  useEffect(() => {
    if (!menuOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (!menuWrapRef.current?.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setMenuOpen(false);
      }
    };
    document.addEventListener("click", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("click", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  return (
    <li ref={setDropRef} style={style}>
      <div className={`editor-node-row${selected ? " is-selected" : ""}`}>
        <button type="button" className="editor-node-select" onClick={onSelect}>
          <span className="editor-node-main">
            <span className="editor-node-name">{node.name}</span>
            <span className="editor-node-trigger">{getPrimaryTriggerPhrase(node)}</span>
          </span>
        </button>

        <div className="editor-node-actions">
          <button
            type="button"
            className="editor-drag-handle editor-node-drag-handle"
            ref={setDragRef}
            aria-label={`Drag to reorder ${node.name}`}
            onClick={(e) => e.stopPropagation()}
            {...listeners}
            {...attributes}
          >
            ⠿
          </button>
          <div className="editor-node-menu-wrap" ref={menuWrapRef}>
            <button
              type="button"
              className="editor-node-more-btn"
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              aria-label={`More actions for ${node.name}`}
              onClick={(e) => {
                e.stopPropagation();
                setMenuOpen((o) => !o);
              }}
            >
              ···
            </button>
            {menuOpen && (
              <ul className="editor-node-menu" role="menu">
                <li role="none">
                  <button
                    type="button"
                    role="menuitem"
                    className="editor-node-menu-item"
                    onClick={(e) => {
                      e.stopPropagation();
                      onMoveUp();
                      setMenuOpen(false);
                    }}
                  >
                    Move up
                  </button>
                </li>
                <li role="none">
                  <button
                    type="button"
                    role="menuitem"
                    className="editor-node-menu-item"
                    onClick={(e) => {
                      e.stopPropagation();
                      onMoveDown();
                      setMenuOpen(false);
                    }}
                  >
                    Move down
                  </button>
                </li>
                <li role="none">
                  <button
                    type="button"
                    role="menuitem"
                    className="editor-node-menu-item"
                    onClick={(e) => {
                      e.stopPropagation();
                      onToggle();
                      setMenuOpen(false);
                    }}
                  >
                    {node.enabled ? "Turn off" : "Turn on"}
                  </button>
                </li>
                <li role="separator" className="editor-node-menu-sep" />
                <li role="none">
                  <button
                    type="button"
                    role="menuitem"
                    className="editor-node-menu-item editor-node-menu-item--danger"
                    onClick={(e) => {
                      e.stopPropagation();
                      setMenuOpen(false);
                      onDelete();
                    }}
                  >
                    Delete…
                  </button>
                </li>
              </ul>
            )}
          </div>
        </div>
      </div>
    </li>
  );
}

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
import { useEffect, useRef, useState } from "react";
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

  const persistReorder = (orderedIds: number[]) => {
    const previousNodes = useEditorStore.getState().nodes;
    reorderNodes(orderedIds);
    void invoke("reorder_commands", { payload: { orderedIds } }).catch((err: unknown) => {
      setNodes(previousNodes);
      showError(`Failed to reorder commands: ${String(err)}`);
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
  const rowId = makeNodeRowId(node.id);
  const { setNodeRef: setDropRef } = useDroppable({ id: rowId });
  const { attributes, listeners, setNodeRef: setDragRef, transform } = useDraggable({
    id: rowId,
  });
  const style = { transform: CSS.Translate.toString(transform) };
  return (
    <li ref={setDropRef} style={style}>
      <div
        className={`editor-node-row${selected ? " is-selected" : ""}`}
        onClick={onSelect}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onSelect();
          }
        }}
        role="button"
        tabIndex={0}
      >
        <span className="editor-node-main">
          <span className="editor-node-name">{node.name}</span>
          <span className="editor-node-trigger">{getPrimaryTriggerPhrase(node)}</span>
        </span>

        <span className="editor-node-actions">
          <button
            type="button"
            className="editor-drag-handle editor-node-drag-handle"
            ref={setDragRef}
            aria-label={`Drag ${node.name}`}
            onClick={(e) => e.stopPropagation()}
            {...listeners}
            {...attributes}
          >
            ⠿
          </button>
          <button
            type="button"
            className="editor-move-btn"
            aria-label={`Move ${node.name} up`}
            onClick={(e) => {
              e.stopPropagation();
              onMoveUp();
            }}
          >
            ↑
          </button>
          <button
            type="button"
            className="editor-move-btn"
            aria-label={`Move ${node.name} down`}
            onClick={(e) => {
              e.stopPropagation();
              onMoveDown();
            }}
          >
            ↓
          </button>
          <button
            type="button"
            className={`editor-toggle${node.enabled ? " is-on" : ""}`}
            aria-pressed={node.enabled}
            aria-label={node.enabled ? "Disable node" : "Enable node"}
            onClick={(e) => {
              e.stopPropagation();
              onToggle();
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
              onDelete();
            }}
          >
            Delete
          </button>
        </span>
      </div>
    </li>
  );
}

import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import { CSS } from "@dnd-kit/utilities";
import { useMemo } from "react";
import type { FormActionPayload } from "../../types";
import { editorPendingAction } from "../../types";
import { ActionCard } from "./ActionCard";
import {
  makeRowId,
  moveByArrow,
  reorderActionsFromDrag,
} from "./ActionChain.logic";

type ActionChainProps = {
  title: string;
  actions: FormActionPayload[];
  onChange: (next: FormActionPayload[]) => void;
  errorByIndex?: Record<number, string>;
};

export function ActionChain({ title, actions, onChange, errorByIndex }: ActionChainProps) {
  const sensors = useSensors(useSensor(PointerSensor), useSensor(KeyboardSensor));

  const items = useMemo(
    () =>
      actions.map((action, index) => ({
        id: makeRowId(index),
        action,
        index,
      })),
    [actions],
  );

  const addAction = () => {
    onChange([...actions, editorPendingAction()]);
  };

  const removeAction = (index: number) => {
    onChange(actions.filter((_, idx) => idx !== index));
  };

  const moveRowByArrow = (index: number, direction: -1 | 1) => {
    onChange(moveByArrow(actions, index, direction));
  };

  const onDragEnd = (event: DragEndEvent) => {
    if (!event.over) return;
    const next = reorderActionsFromDrag(actions, String(event.active.id), String(event.over.id));
    if (next) onChange(next);
  };

  return (
    <section className="editor-action-chain">
      <div className="editor-chain-header">
        <h3>{title}</h3>
        <button type="button" className="editor-add-btn" onClick={addAction} aria-label={`Add ${title} action`}>
          +
        </button>
      </div>

      {actions.length === 0 && <p className="editor-chain-empty">No actions yet.</p>}

      <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
        <ul className="editor-chain-list">
          {items.map((item) => (
            <ActionRow
              key={item.id}
              id={item.id}
              chainTitle={title}
              index={item.index}
              action={item.action}
              onRemove={() => removeAction(item.index)}
              onChange={(nextAction) => {
                const nextActions = [...actions];
                nextActions[item.index] = nextAction;
                onChange(nextActions);
              }}
              onMoveUp={() => moveRowByArrow(item.index, -1)}
              onMoveDown={() => moveRowByArrow(item.index, 1)}
              errorText={errorByIndex?.[item.index]}
            />
          ))}
        </ul>
      </DndContext>
    </section>
  );
}

type ActionRowProps = {
  id: string;
  chainTitle: string;
  index: number;
  action: FormActionPayload;
  onChange: (next: FormActionPayload) => void;
  onRemove: () => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
  errorText?: string;
};

function ActionRow({
  id,
  chainTitle,
  index,
  action,
  onChange,
  onRemove,
  onMoveUp,
  onMoveDown,
  errorText,
}: ActionRowProps) {
  const { setNodeRef: setDropRef } = useDroppable({ id });
  const { attributes, listeners, setNodeRef: setDragRef, transform } = useDraggable({ id });
  const style = {
    transform: CSS.Translate.toString(transform),
  };
  return (
    <li ref={setDropRef} className="editor-chain-row" style={style}>
      <div className="editor-chain-row-controls">
        <button
          type="button"
          className="editor-drag-handle"
          ref={setDragRef}
          aria-label={`Drag to reorder ${chainTitle} step ${index + 1}`}
          {...listeners}
          {...attributes}
        >
          ⠿
        </button>
        <button type="button" className="editor-move-btn" onClick={onMoveUp} aria-label="Move action up">
          ↑
        </button>
        <button type="button" className="editor-move-btn" onClick={onMoveDown} aria-label="Move action down">
          ↓
        </button>
      </div>
      <div className="editor-chain-row-body">
        <ActionCard action={action} index={index} onChange={onChange} onRemove={onRemove} />
        {errorText && <p className="editor-field-error">{errorText}</p>}
      </div>
    </li>
  );
}


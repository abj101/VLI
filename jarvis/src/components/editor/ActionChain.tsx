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
import type { ActionPayload } from "../../types";
import { ActionCard } from "./ActionCard";
import { defaultActionForKind } from "./NodeForm.logic";

type ActionChainProps = {
  title: string;
  actions: ActionPayload[];
  onChange: (next: ActionPayload[]) => void;
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
    onChange([...actions, defaultActionForKind("open_app")]);
  };

  const removeAction = (index: number) => {
    onChange(actions.filter((_, idx) => idx !== index));
  };

  const moveByArrow = (index: number, direction: -1 | 1) => {
    const nextIndex = index + direction;
    if (nextIndex < 0 || nextIndex >= actions.length) return;
    const nextActions = [...actions];
    const [removed] = nextActions.splice(index, 1);
    nextActions.splice(nextIndex, 0, removed);
    onChange(nextActions);
  };

  const onDragEnd = (event: DragEndEvent) => {
    if (!event.over) return;
    const oldIndex = parseRowId(String(event.active.id));
    const newIndex = parseRowId(String(event.over.id));
    if (oldIndex === -1 || newIndex === -1 || oldIndex === newIndex) {
      return;
    }
    const nextActions = [...actions];
    const [removed] = nextActions.splice(oldIndex, 1);
    nextActions.splice(newIndex, 0, removed);
    onChange(nextActions);
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
              index={item.index}
              action={item.action}
              onRemove={() => removeAction(item.index)}
              onChange={(nextAction) => {
                const nextActions = [...actions];
                nextActions[item.index] = nextAction;
                onChange(nextActions);
              }}
              onMoveUp={() => moveByArrow(item.index, -1)}
              onMoveDown={() => moveByArrow(item.index, 1)}
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
  index: number;
  action: ActionPayload;
  onChange: (next: ActionPayload) => void;
  onRemove: () => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
  errorText?: string;
};

function ActionRow({
  id,
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
        <button type="button" className="editor-drag-handle" ref={setDragRef} {...listeners} {...attributes}>
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

function makeRowId(index: number): string {
  return `row-${index}`;
}

function parseRowId(id: string): number {
  const value = Number(id.replace("row-", ""));
  return Number.isInteger(value) ? value : -1;
}

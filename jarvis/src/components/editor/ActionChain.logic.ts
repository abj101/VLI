import type { ActionPayload } from "../../types";

/** Stable row id for @dnd-kit (index-based). */
export function makeRowId(index: number): string {
  return `row-${index}`;
}

export function parseRowId(id: string): number {
  const value = Number(id.replace("row-", ""));
  return Number.isInteger(value) ? value : -1;
}

/** Reorder array by moving one item from oldIndex to newIndex (inclusive bounds). */
export function reorderByMove<T>(items: readonly T[], oldIndex: number, newIndex: number): T[] {
  if (
    oldIndex === newIndex ||
    oldIndex < 0 ||
    newIndex < 0 ||
    oldIndex >= items.length ||
    newIndex >= items.length
  ) {
    return [...items];
  }
  const next = [...items];
  const [removed] = next.splice(oldIndex, 1);
  next.splice(newIndex, 0, removed);
  return next;
}

/** Move item at index one step up or down; no-op at edges. */
export function moveByArrow<T>(items: readonly T[], index: number, direction: -1 | 1): T[] {
  const nextIndex = index + direction;
  if (nextIndex < 0 || nextIndex >= items.length) {
    return [...items];
  }
  return reorderByMove(items, index, nextIndex);
}

/** Apply drag-end when both active and over ids are row ids; returns null if invalid or no-op. */
export function reorderActionsFromDrag(
  actions: ActionPayload[],
  activeId: string,
  overId: string,
): ActionPayload[] | null {
  const oldIndex = parseRowId(activeId);
  const newIndex = parseRowId(overId);
  if (oldIndex === -1 || newIndex === -1 || oldIndex === newIndex) {
    return null;
  }
  return reorderByMove(actions, oldIndex, newIndex);
}

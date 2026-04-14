import type { CommandNodePayload } from "../../types";

export function getPrimaryTriggerPhrase(node: CommandNodePayload): string {
  return node.trigger_phrases[0] ?? "(no trigger phrase)";
}

export function withEnabledValue(
  node: CommandNodePayload,
  enabled: boolean,
): CommandNodePayload {
  return { ...node, enabled };
}

export function makeNodeRowId(id: number): string {
  return `node-${id}`;
}

export function parseNodeRowId(value: string): number {
  const parsed = Number(value.replace("node-", ""));
  return Number.isInteger(parsed) ? parsed : -1;
}

export function reorderIdsByDrag(
  currentIds: number[],
  activeId: number,
  overId: number,
): number[] {
  const oldIndex = currentIds.indexOf(activeId);
  const newIndex = currentIds.indexOf(overId);
  if (oldIndex < 0 || newIndex < 0 || oldIndex === newIndex) {
    return currentIds;
  }
  const next = [...currentIds];
  const [removed] = next.splice(oldIndex, 1);
  next.splice(newIndex, 0, removed);
  return next;
}

export function reorderIdsByArrow(
  currentIds: number[],
  id: number,
  direction: -1 | 1,
): number[] {
  const currentIndex = currentIds.indexOf(id);
  const nextIndex = currentIndex + direction;
  if (currentIndex < 0 || nextIndex < 0 || nextIndex >= currentIds.length) {
    return currentIds;
  }
  const next = [...currentIds];
  const [removed] = next.splice(currentIndex, 1);
  next.splice(nextIndex, 0, removed);
  return next;
}

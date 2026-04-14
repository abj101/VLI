import { create } from "zustand";
import type { CommandNodePayload } from "../types";

export type EditorStore = {
  nodes: CommandNodePayload[];
  selectedId: number | null;
  setSelected: (id: number | null) => void;
  setNodes: (nodes: CommandNodePayload[]) => void;
  reorderNodes: (orderedIds: number[]) => void;
  deleteNode: (id: number) => void;
  toggleEnabled: (id: number) => void;
};

export const useEditorStore = create<EditorStore>((set) => ({
  nodes: [],
  selectedId: null,
  setSelected(id) {
    set({ selectedId: id });
  },
  setNodes(nodes) {
    set({ nodes });
  },
  reorderNodes(orderedIds) {
    set((state) => {
      if (orderedIds.length === 0) {
        return { nodes: state.nodes };
      }
      const idOrder = new Map(orderedIds.map((id, index) => [id, index]));
      const tailStart = orderedIds.length;
      return {
        nodes: [...state.nodes].sort((left, right) => {
          const leftOrder = idOrder.get(left.id) ?? tailStart;
          const rightOrder = idOrder.get(right.id) ?? tailStart;
          if (leftOrder !== rightOrder) return leftOrder - rightOrder;
          return left.id - right.id;
        }),
      };
    });
  },
  deleteNode(id) {
    set((state) => ({
      nodes: state.nodes.filter((node) => node.id !== id),
      selectedId: state.selectedId === id ? null : state.selectedId,
    }));
  },
  toggleEnabled(id) {
    set((state) => ({
      nodes: state.nodes.map((node) =>
        node.id === id ? { ...node, enabled: !node.enabled } : node,
      ),
    }));
  },
}));

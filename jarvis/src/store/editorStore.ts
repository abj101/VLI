import { create } from "zustand";
import type { CommandNodePayload } from "../types";

export type EditorStore = {
  nodes: CommandNodePayload[];
  selectedId: number | null;
  setSelected: (id: number | null) => void;
  setNodes: (nodes: CommandNodePayload[]) => void;
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

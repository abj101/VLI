import { create } from "zustand";
import type { CommandNodePayload } from "../types";

export type EditorStore = {
  nodes: CommandNodePayload[];
  setNodes: (nodes: CommandNodePayload[]) => void;
  deleteNode: (id: number) => void;
  toggleEnabled: (id: number) => void;
};

export const useEditorStore = create<EditorStore>((set) => ({
  nodes: [],
  setNodes(nodes) {
    set({ nodes });
  },
  deleteNode(id) {
    set((state) => ({
      nodes: state.nodes.filter((node) => node.id !== id),
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

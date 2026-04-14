import { create } from "zustand";

export type SettingsUiState = {
  appIndexCount: number | null;
  setAppIndexCount: (count: number) => void;
};

export const useSettingsStore = create<SettingsUiState>((set) => ({
  appIndexCount: null,
  setAppIndexCount(count) {
    set({ appIndexCount: count });
  },
}));

import { create } from "zustand";

type SettingsUiState = {
  appIndexCount: number | null;
  appIndexScanning: boolean;
  setAppIndexStatus: (status: { count: number; scanning: boolean }) => void;
};

export const useSettingsStore = create<SettingsUiState>((set) => ({
  appIndexCount: null,
  appIndexScanning: false,
  setAppIndexStatus({ count, scanning }) {
    set({ appIndexCount: count, appIndexScanning: scanning });
  },
}));

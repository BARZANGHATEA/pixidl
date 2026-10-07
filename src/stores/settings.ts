// Settings state. Every change is saved immediately and applied by the backend.
import { create } from "zustand";
import { api } from "../services/api";
import type { Category, Settings } from "../types";

interface SettingsState {
  settings: Settings | null;
  categories: Category[];
  problems: string[];
  saving: boolean;
  load: () => Promise<void>;
  update: (patch: Partial<Settings>) => Promise<void>;
  loadCategories: () => Promise<void>;
}

export const useSettings = create<SettingsState>((set, get) => ({
  settings: null,
  categories: [],
  problems: [],
  saving: false,
  load: async () => {
    const [settings, categories] = await Promise.all([api.getSettings(), api.getCategories()]);
    set({ settings, categories });
  },
  update: async (patch) => {
    const current = get().settings;
    if (!current) return;
    const next = { ...current, ...patch };
    set({ settings: next, saving: true });
    try {
      const problems = await api.updateSettings(next);
      // Re-read: the backend may have corrected invalid values.
      const saved = await api.getSettings();
      set({ settings: saved, problems, saving: false });
    } catch (e) {
      set({ settings: current, saving: false, problems: [(e as { message?: string }).message ?? "Could not save settings"] });
    }
  },
  loadCategories: async () => set({ categories: await api.getCategories() }),
}));

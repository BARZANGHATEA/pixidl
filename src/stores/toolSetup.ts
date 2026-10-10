import { create } from "zustand";
import type { ToolSetup } from "../types/generated/ToolSetup";

/** The external tool currently being downloaded for the video engine, if any. */
export const useToolSetup = create<{ current: ToolSetup | null; set: (s: ToolSetup | null) => void }>((set) => ({
  current: null,
  set: (current) => set({ current }),
}));

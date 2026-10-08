// UI-only state: navigation, filters, dialogs and toasts.
import { create } from "zustand";
import type { CommandError, EngineKind } from "../types";
import type { Scope, SortKey, StatusFilter } from "../lib/filters";
import type { PowerCountdown } from "../services/events";

export type View = "downloads" | "history" | "settings" | "about";

export interface Toast {
  id: number;
  tone: "success" | "error" | "info";
  title: string;
  body?: string;
  detail?: string | null;
  action?: { label: string; run: () => void };
}

export interface ConfirmRequest {
  title: string;
  body: string;
  confirmLabel: string;
  danger?: boolean;
  checkbox?: string;
  onConfirm: (checked: boolean) => void;
}

const SORT_KEY = "pixidl.sort";
function initialSort(): SortKey {
  try {
    const v = localStorage.getItem(SORT_KEY);
    if (v && ["newest", "oldest", "name", "size", "progress", "speed"].includes(v)) return v as SortKey;
  } catch {
    /* storage unavailable */
  }
  return "newest";
}

interface UiState {
  view: View;
  scope: Scope;
  status: StatusFilter;
  query: string;
  sort: SortKey;
  addDialog: { open: boolean; url: string; engine: EngineKind | null };
  detailsId: string | null;
  toasts: Toast[];
  confirm: ConfirmRequest | null;
  power: PowerCountdown | null;
  setView: (v: View) => void;
  setScope: (s: Scope) => void;
  setStatus: (s: StatusFilter) => void;
  setQuery: (q: string) => void;
  setSort: (s: SortKey) => void;
  openAdd: (url?: string, engine?: EngineKind | null) => void;
  closeAdd: () => void;
  showDetails: (id: string | null) => void;
  toast: (t: Omit<Toast, "id">) => void;
  toastError: (title: string, e: CommandError | { message: string; detail?: string | null }) => void;
  dismissToast: (id: number) => void;
  ask: (c: ConfirmRequest | null) => void;
  setPower: (p: PowerCountdown | null) => void;
}

let toastSeq = 1;

export const useUi = create<UiState>((set, get) => ({
  view: "downloads",
  scope: { kind: "all" },
  status: "all",
  query: "",
  sort: initialSort(),
  addDialog: { open: false, url: "", engine: null },
  detailsId: null,
  toasts: [],
  confirm: null,
  power: null,
  setView: (view) => set({ view }),
  setScope: (scope) => set({ scope, view: "downloads" }),
  setStatus: (status) => set({ status, view: "downloads" }),
  setQuery: (query) => set({ query }),
  setSort: (sort) => {
    try {
      localStorage.setItem(SORT_KEY, sort);
    } catch {
      /* ignore */
    }
    set({ sort });
  },
  openAdd: (url = "", engine = null) => set({ addDialog: { open: true, url, engine } }),
  closeAdd: () => set({ addDialog: { open: false, url: "", engine: null } }),
  showDetails: (detailsId) => set({ detailsId }),
  toast: (t) => {
    const id = toastSeq++;
    set({ toasts: [...get().toasts.slice(-3), { ...t, id }] });
    const ttl = t.tone === "error" ? 9000 : 5000;
    setTimeout(() => get().dismissToast(id), ttl);
  },
  toastError: (title, e) => get().toast({ tone: "error", title, body: e.message, detail: e.detail ?? null }),
  dismissToast: (id) => set({ toasts: get().toasts.filter((t) => t.id !== id) }),
  ask: (confirm) => set({ confirm }),
  setPower: (power) => set({ power }),
}));

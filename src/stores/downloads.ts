// Download list state, kept in sync with the backend through ManagerEvents.
import { create } from "zustand";
import { api } from "../services/api";
import type { Download, ManagerEvent } from "../types";

interface DownloadsState {
  byId: Record<string, Download>;
  loaded: boolean;
  error: string | null;
  load: () => Promise<void>;
  applyEvent: (e: ManagerEvent) => void;
  list: () => Download[];
}

export function reduceEvent(byId: Record<string, Download>, e: ManagerEvent): Record<string, Download> {
  switch (e.type) {
    case "download_created":
    case "download_updated":
    case "download_completed":
    case "download_failed":
      return { ...byId, [e.download.id]: e.download };
    case "download_removed": {
      if (!(e.id in byId)) return byId;
      const next = { ...byId };
      delete next[e.id];
      return next;
    }
    case "download_progress": {
      let next = byId;
      for (const u of e.updates) {
        const d = next[u.downloadId];
        if (!d) continue;
        if (next === byId) next = { ...byId };
        next[u.downloadId] = {
          ...d,
          status: u.status,
          downloadedBytes: u.downloadedBytes,
          totalBytes: u.totalBytes,
          speedBps: u.speedBytesPerSecond,
          uploadBps: u.uploadBytesPerSecond,
          etaSeconds: u.etaSeconds,
          peers: u.peers,
          seeds: u.seeds,
        };
      }
      return next;
    }
    default:
      return byId;
  }
}

export const useDownloads = create<DownloadsState>((set, get) => ({
  byId: {},
  loaded: false,
  error: null,
  load: async () => {
    try {
      const list = await api.listDownloads();
      set({ byId: Object.fromEntries(list.map((d) => [d.id, d])), loaded: true, error: null });
    } catch (e) {
      set({ loaded: true, error: (e as { message?: string }).message ?? "Failed to load downloads" });
    }
  },
  applyEvent: (e) => set((s) => ({ byId: reduceEvent(s.byId, e) })),
  list: () => Object.values(get().byId),
}));

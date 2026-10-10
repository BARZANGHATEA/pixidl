// App update state: the last check, the download in progress and the
// verified installer. Lives for the whole session so leaving the Updates page
// doesn't lose a running download.
import { create } from "zustand";
import { api, toCommandError } from "../services/api";
import type { CommandError, UpdateEvent, UpdateStatus } from "../types";

export const REPO_URL = "https://github.com/BARZANGHATEA/pixidl";
export const RELEASES_URL = `${REPO_URL}/releases`;

export type UpdatePhase = "idle" | "checking" | "downloading" | "verifying" | "ready" | "installing";

export interface UpdateProgress {
  downloaded: number;
  total: number | null;
  speedBps: number;
}

export interface UpdateError extends CommandError {
  during: "check" | "download" | "install";
}

interface UpdatesState {
  status: UpdateStatus | null;
  phase: UpdatePhase;
  progress: UpdateProgress | null;
  error: UpdateError | null;
  load: () => Promise<void>;
  check: () => Promise<void>;
  download: () => Promise<void>;
  cancel: () => Promise<void>;
  install: () => Promise<void>;
  applyEvent: (e: UpdateEvent) => void;
}

function phaseOf(s: UpdateStatus): UpdatePhase {
  if (s.readyVersion) return "ready";
  if (s.downloading) return "downloading";
  return "idle";
}

export const useUpdates = create<UpdatesState>((set, get) => ({
  status: null,
  phase: "idle",
  progress: null,
  error: null,
  load: async () => {
    try {
      const status = await api.getUpdateStatus();
      const busy = get().phase === "checking" || get().phase === "installing";
      set({ status, phase: busy ? get().phase : phaseOf(status) });
    } catch {
      /* not running inside the app */
    }
  },
  check: async () => {
    if (get().phase === "checking") return;
    set({ phase: "checking", error: null });
    try {
      const status = await api.checkForUpdates();
      set({ status, phase: phaseOf(status) });
    } catch (e) {
      set({ phase: "idle", error: { ...toCommandError(e), during: "check" } });
    }
  },
  download: async () => {
    const size = get().status?.info?.latest?.installer?.size ?? null;
    set({ phase: "downloading", error: null, progress: { downloaded: 0, total: size, speedBps: 0 } });
    try {
      await api.downloadUpdate();
      const status = await api.getUpdateStatus().catch(() => get().status);
      set({ status, phase: "ready", progress: null });
    } catch (e) {
      const err = toCommandError(e);
      // A cancel isn't an error: back to "update available".
      set({ phase: "idle", progress: null, error: err.kind === "cancelled" ? null : { ...err, during: "download" } });
    }
  },
  cancel: async () => {
    await api.cancelUpdateDownload().catch(() => false);
  },
  install: async () => {
    set({ phase: "installing", error: null });
    try {
      await api.installUpdate();
      // The app exits now; the installer restarts it.
    } catch (e) {
      set({ phase: "ready", error: { ...toCommandError(e), during: "install" } });
    }
  },
  applyEvent: (e) => {
    const { status, phase } = get();
    switch (e.type) {
      case "available":
        if (status) set({ status: { ...status, info: e.info } });
        else void get().load();
        break;
      case "progress":
        set({ progress: { downloaded: e.downloaded, total: e.total, speedBps: e.speedBps }, phase: phase === "verifying" ? phase : "downloading" });
        break;
      case "verifying":
        set({ phase: "verifying" });
        break;
      case "ready":
        set({ phase: "ready", progress: null, status: status ? { ...status, readyVersion: e.version, downloading: false } : status });
        break;
    }
  },
}));

/** True when a newer version than the running one is published. */
export const selectUpdateAvailable = (s: UpdatesState) => !!s.status?.info?.updateAvailable;

// Typed wrappers around the Tauri IPC commands (src-tauri/src/commands.rs).
// The UI never talks to `invoke` directly.
import { invoke } from "@tauri-apps/api/core";
import type {
  AddDownloadRequest,
  AppInfo,
  BrowserIntegrationStatus,
  BrowserRegistration,
  Category,
  CommandError,
  Download,
  DownloadEvent,
  EngineKind,
  EngineStatus,
  GlobalStats,
  Priority,
  Settings,
  TorrentFilePreview,
  UrlInspection,
} from "../types";

export function isCommandError(e: unknown): e is CommandError {
  return typeof e === "object" && e !== null && "message" in e && "kind" in e;
}

/** Normalises anything thrown by `invoke` into a CommandError. */
export function toCommandError(e: unknown): CommandError {
  if (isCommandError(e)) return e;
  return { kind: "unknown", message: typeof e === "string" ? e : e instanceof Error ? e.message : "Unexpected error", detail: null };
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toCommandError(e);
  }
}

export const api = {
  listDownloads: () => call<Download[]>("list_downloads"),
  getDownload: (id: string) => call<Download | null>("get_download", { id }),
  getDownloadEvents: (id: string) => call<DownloadEvent[]>("get_download_events", { id }),
  getStats: () => call<GlobalStats>("get_stats"),
  addDownload: (request: AddDownloadRequest) => call<Download>("add_download", { request }),
  inspectUrl: (url: string, engine?: EngineKind | null) => call<UrlInspection>("inspect_url", { url, engine: engine ?? null }),
  readTorrentFile: (path: string) => call<TorrentFilePreview>("read_torrent_file", { path }),
  pause: (id: string) => call<void>("pause_download", { id }),
  resume: (id: string) => call<void>("resume_download", { id }),
  retry: (id: string) => call<void>("retry_download", { id }),
  cancel: (id: string) => call<void>("cancel_download", { id }),
  remove: (id: string, deleteFiles: boolean) => call<void>("remove_download", { id, deleteFiles }),
  pauseAll: () => call<void>("pause_all"),
  resumeAll: () => call<void>("resume_all"),
  reorderQueue: (ids: string[]) => call<void>("reorder_queue", { ids }),
  setPriority: (id: string, priority: Priority) => call<void>("set_priority", { id, priority }),
  setCategory: (id: string, category: string) => call<void>("set_category", { id, category }),
  setDownloadLimit: (id: string, limit: number | null) => call<void>("set_download_limit", { id, limit }),
  scheduleDownload: (id: string, at: string | null) => call<void>("schedule_download", { id, at }),
  setGlobalLimit: (limit: number | null) => call<void>("set_global_limit", { limit }),
  clearHistory: () => call<void>("clear_history"),
  verifyFiles: () => call<void>("verify_files"),
  openFile: (id: string) => call<void>("open_file", { id }),
  openFolder: (id: string) => call<void>("open_folder", { id }),
  openDownloadsFolder: () => call<void>("open_downloads_folder"),
  openLogsFolder: () => call<void>("open_logs_folder"),
  getSettings: () => call<Settings>("get_settings"),
  updateSettings: (settings: Settings) => call<string[]>("update_settings", { settings }),
  completeFirstRun: (settings: Settings) => call<string[]>("complete_first_run", { settings }),
  getCategories: () => call<Category[]>("get_categories"),
  upsertCategory: (category: Category) => call<void>("upsert_category", { category }),
  deleteCategory: (name: string) => call<void>("delete_category", { name }),
  getEngineStatus: () => call<EngineStatus>("get_engine_status"),
  installYtdlp: () => call<string>("install_ytdlp"),
  updateYtdlp: () => call<string>("update_ytdlp"),
  getBrowserIntegration: () => call<BrowserIntegrationStatus>("get_browser_integration"),
  reinstallBrowserIntegration: () => call<BrowserRegistration[]>("reinstall_browser_integration"),
  getAppInfo: () => call<AppInfo>("get_app_info"),
  getLicenses: () => call<string>("get_licenses"),
  cancelPowerAction: () => call<boolean>("cancel_power_action"),
  quit: () => call<void>("quit_app"),
};

export type Api = typeof api;

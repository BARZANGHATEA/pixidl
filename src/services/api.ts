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
  ExtensionBrowser,
  PreparedExtension,
  GlobalStats,
  LinkProbe,
  Priority,
  Queue,
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
  clearCompleted: () => call<number>("clear_completed_downloads"),
  resumeMany: (ids: string[]) => call<void>("resume_downloads", { ids }),
  pauseMany: (ids: string[]) => call<void>("pause_downloads", { ids }),
  removeMany: (ids: string[], deleteFiles: boolean) => call<void>("remove_downloads", { ids, deleteFiles }),
  moveToQueue: (ids: string[], queueId: string) => call<void>("move_to_queue", { ids, queueId }),
  listQueues: () => call<Queue[]>("list_queues"),
  createQueue: (name: string, maxConcurrent: number) => call<Queue>("create_queue", { name, maxConcurrent }),
  renameQueue: (id: string, name: string) => call<void>("rename_queue", { id, name }),
  setQueueMaxConcurrent: (id: string, maxConcurrent: number) => call<void>("set_queue_max_concurrent", { id, maxConcurrent }),
  deleteQueue: (id: string) => call<void>("delete_queue", { id }),
  startQueue: (id: string) => call<void>("start_queue", { id }),
  stopQueue: (id: string) => call<void>("stop_queue", { id }),
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
  getExtensionBrowsers: () => call<ExtensionBrowser[]>("get_extension_browsers"),
  getExtension: (browserName: string) => call<PreparedExtension>("get_extension", { browserName }),
  openBrowserExtensionsPage: (browserName: string) => call<void>("open_browser_extensions_page", { browserName }),
  revealPath: (path: string) => call<void>("reveal_path", { path }),
  installDeno: () => call<string>("install_deno"),
  installFfmpeg: () => call<string>("install_ffmpeg"),
  setNativeLabels: (labels: Record<string, string>) => call<void>("set_native_labels", { labels }),
  probeLinks: (urls: string[]) => call<LinkProbe[]>("probe_links", { urls }),
};

export type Api = typeof api;

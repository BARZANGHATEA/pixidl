// Shared types. Everything under ./generated is produced from the Rust
// structs by ts-rs (`npm run gen:types`) — never edit those by hand.
export type { AddDownloadRequest } from "./generated/AddDownloadRequest";
export type { AfterQueueAction } from "./generated/AfterQueueAction";
export type { Browser } from "./generated/Browser";
export type { BrowserRegistration } from "./generated/BrowserRegistration";
export type { Category } from "./generated/Category";
export type { CloseBehavior } from "./generated/CloseBehavior";
export type { Download } from "./generated/Download";
export type { DownloadEvent } from "./generated/DownloadEvent";
export type { DownloadStatus } from "./generated/DownloadStatus";
export type { DuplicatePolicy } from "./generated/DuplicatePolicy";
export type { EngineKind } from "./generated/EngineKind";
export type { EngineOptions } from "./generated/EngineOptions";
export type { ErrorKind } from "./generated/ErrorKind";
export type { FormatPreset } from "./generated/FormatPreset";
export type { GlobalStats } from "./generated/GlobalStats";
export type { InspectionError } from "./generated/InspectionError";
export type { ManagerEvent } from "./generated/ManagerEvent";
export type { Priority } from "./generated/Priority";
export type { ProgressUpdate } from "./generated/ProgressUpdate";
export type { ProxyMode } from "./generated/ProxyMode";
export type { ScheduleSettings } from "./generated/ScheduleSettings";
export type { Settings } from "./generated/Settings";
export type { Theme } from "./generated/Theme";
export type { ToolStatus } from "./generated/ToolStatus";
export type { TorrentEngineStatus } from "./generated/TorrentEngineStatus";
export type { TorrentFile } from "./generated/TorrentFile";
export type { TorrentInfo } from "./generated/TorrentInfo";
export type { UrlInspection } from "./generated/UrlInspection";
export type { VideoFormat } from "./generated/VideoFormat";
export type { VideoInfo } from "./generated/VideoInfo";

import type { ErrorKind } from "./generated/ErrorKind";
import type { ToolStatus } from "./generated/ToolStatus";
import type { TorrentEngineStatus } from "./generated/TorrentEngineStatus";
import type { TorrentInfo } from "./generated/TorrentInfo";
import type { BrowserRegistration } from "./generated/BrowserRegistration";

/** Error shape returned by every failing command (see src-tauri/src/error.rs). */
export interface CommandError {
  kind: ErrorKind;
  message: string;
  detail: string | null;
}

export interface EngineStatus {
  ytdlp: ToolStatus;
  ffmpeg: ToolStatus;
  torrent: TorrentEngineStatus;
  http: ToolStatus;
}

export interface TorrentFilePreview {
  info: TorrentInfo;
  base64: string;
  fileName: string;
}

export interface BrowserIntegrationStatus {
  enabled: boolean;
  bridgeRunning: boolean;
  hostPath: string | null;
  hostInstalled: boolean;
  browsers: BrowserRegistration[];
  referenceExtensionId: string;
  protocolVersion: number;
}

export interface AppInfo {
  name: string;
  version: string;
  identifier: string;
  dataDir: string;
  logsDir: string;
  platform: string;
  arch: string;
  protocolVersion: number;
  tauriVersion: string;
}

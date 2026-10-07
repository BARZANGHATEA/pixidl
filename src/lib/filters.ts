// Pure filtering, searching and sorting of the download list.
import type { Download, DownloadStatus, EngineKind } from "../types";

export type StatusFilter = "all" | "active" | "completed" | "failed";
export type Scope = { kind: "all" } | { kind: "engine"; engine: EngineKind } | { kind: "category"; name: string };
export type SortKey = "newest" | "oldest" | "name" | "size" | "progress" | "speed";

export const ACTIVE_STATUSES: DownloadStatus[] = ["queued", "preparing", "downloading", "paused"];

export function matchesStatus(d: Download, f: StatusFilter): boolean {
  switch (f) {
    case "all":
      return true;
    case "active":
      return ACTIVE_STATUSES.includes(d.status);
    case "completed":
      return d.status === "completed";
    case "failed":
      return d.status === "failed" || d.status === "cancelled";
  }
}

export function matchesScope(d: Download, s: Scope): boolean {
  if (s.kind === "all") return true;
  if (s.kind === "engine") return d.engine === s.engine;
  return d.category === s.name;
}

/** Case-insensitive search over filename, title, URL, category and status. */
export function matchesSearch(d: Download, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const hay = [d.filename, d.title ?? "", d.url, d.originalUrl, d.category, d.status].join("\n").toLowerCase();
  return q.split(/\s+/).every((term) => hay.includes(term));
}

function progressOf(d: Download): number {
  if (d.status === "completed") return 1;
  return d.totalBytes ? d.downloadedBytes / d.totalBytes : 0;
}

export function sortDownloads(list: Download[], key: SortKey): Download[] {
  const out = [...list];
  const byName = (a: Download, b: Download) => a.filename.localeCompare(b.filename, undefined, { numeric: true, sensitivity: "base" });
  switch (key) {
    case "newest":
      return out.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
    case "oldest":
      return out.sort((a, b) => a.createdAt.localeCompare(b.createdAt));
    case "name":
      return out.sort(byName);
    case "size":
      return out.sort((a, b) => (b.totalBytes ?? -1) - (a.totalBytes ?? -1) || byName(a, b));
    case "progress":
      return out.sort((a, b) => progressOf(b) - progressOf(a) || byName(a, b));
    case "speed":
      return out.sort((a, b) => b.speedBps - a.speedBps || byName(a, b));
  }
}

export function selectVisible(list: Download[], opts: { status: StatusFilter; scope: Scope; query: string; sort: SortKey }): Download[] {
  return sortDownloads(
    list.filter((d) => matchesScope(d, opts.scope) && matchesStatus(d, opts.status) && matchesSearch(d, opts.query)),
    opts.sort,
  );
}

export interface Counts {
  all: number;
  active: number;
  completed: number;
  failed: number;
  torrent: number;
  video: number;
  running: number;
}

export function countDownloads(list: Download[]): Counts {
  const c: Counts = { all: list.length, active: 0, completed: 0, failed: 0, torrent: 0, video: 0, running: 0 };
  for (const d of list) {
    if (matchesStatus(d, "active")) c.active++;
    if (d.status === "completed") c.completed++;
    if (d.status === "failed" || d.status === "cancelled") c.failed++;
    if (d.engine === "torrent") c.torrent++;
    if (d.engine === "video") c.video++;
    if (d.status === "downloading" || d.status === "preparing") c.running++;
  }
  return c;
}

// Test-only factory. Never imported by production code.
import type { Download } from "../types";

let n = 0;
export function makeDownload(over: Partial<Download> = {}): Download {
  n++;
  return {
    id: `id-${n}`,
    url: `https://example.com/file-${n}.zip`,
    originalUrl: `https://example.com/file-${n}.zip`,
    referrer: null,
    filename: `file-${n}.zip`,
    saveDir: "/downloads",
    category: "Archives",
    engine: "http",
    status: "downloading",
    priority: "normal",
    queuePosition: n,
    totalBytes: 1000,
    downloadedBytes: 250,
    speedBps: 100,
    uploadBps: 0,
    etaSeconds: 8,
    resumable: true,
    speedLimitBps: null,
    connections: 4,
    errorKind: null,
    errorMessage: null,
    errorDetail: null,
    retryCount: 0,
    engineOptions: { audioOnly: false, subtitles: false, explicitFilename: false },
    peers: null,
    seeds: null,
    infoHash: null,
    title: null,
    thumbnail: null,
    createdAt: new Date(2026, 0, 1, 0, 0, n).toISOString(),
    startedAt: null,
    completedAt: null,
    updatedAt: new Date(2026, 0, 1).toISOString(),
    scheduledAt: null,
    fileMissing: false,
    queueId: "main",
    ...over,
  };
}

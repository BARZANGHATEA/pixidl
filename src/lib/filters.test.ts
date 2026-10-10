import { countDownloads, matchesSearch, selectVisible, sortDownloads } from "./filters";
import { makeDownload } from "../test/fixtures";

describe("filters", () => {
  const a = makeDownload({ filename: "ubuntu.iso", status: "downloading", totalBytes: 3000, downloadedBytes: 1500, speedBps: 50 });
  const b = makeDownload({ filename: "Movie.mkv", status: "completed", engine: "video", category: "Videos", totalBytes: 9000, downloadedBytes: 9000, speedBps: 0 });
  const c = makeDownload({ filename: "setup.exe", status: "failed", category: "Programs", totalBytes: 100, downloadedBytes: 10, speedBps: 0 });
  const d = makeDownload({ filename: "linux.torrent-data", status: "paused", engine: "torrent", category: "Torrents", totalBytes: null, speedBps: 0, url: "magnet:?xt=urn:btih:x" });
  const list = [a, b, c, d];

  it("filters by status", () => {
    const ids = (s: Parameters<typeof selectVisible>[1]["status"]) => selectVisible(list, { status: s, scope: { kind: "all" }, query: "", sort: "name" }).map((x) => x.filename);
    expect(ids("all")).toHaveLength(4);
    expect(ids("active")).toEqual(["linux.torrent-data", "ubuntu.iso"]);
    expect(ids("completed")).toEqual(["Movie.mkv"]);
    expect(ids("failed")).toEqual(["setup.exe"]);
  });

  it("filters by engine scope and category", () => {
    expect(selectVisible(list, { status: "all", scope: { kind: "engine", engine: "torrent" }, query: "", sort: "name" })).toEqual([d]);
    expect(selectVisible(list, { status: "all", scope: { kind: "category", name: "Videos" }, query: "", sort: "name" })).toEqual([b]);
  });

  it("filters by queue scope", () => {
    const q = makeDownload({ filename: "night.bin", queueId: "night" });
    const withQueue = [...list, q];
    expect(selectVisible(withQueue, { status: "all", scope: { kind: "queue", id: "night" }, query: "", sort: "name" })).toEqual([q]);
    expect(selectVisible(withQueue, { status: "all", scope: { kind: "queue", id: "main" }, query: "", sort: "name" })).toHaveLength(4);
  });

  it("searches filename, URL, category and status", () => {
    expect(matchesSearch(a, "UBUNTU")).toBe(true);
    expect(matchesSearch(d, "magnet")).toBe(true);
    expect(matchesSearch(c, "programs")).toBe(true);
    expect(matchesSearch(c, "failed")).toBe(true);
    expect(matchesSearch(c, "setup programs")).toBe(true);
    expect(matchesSearch(c, "setup videos")).toBe(false);
    expect(matchesSearch(c, "  ")).toBe(true);
  });

  it("sorts", () => {
    expect(sortDownloads(list, "size").map((x) => x.filename)).toEqual(["Movie.mkv", "ubuntu.iso", "setup.exe", "linux.torrent-data"]);
    expect(sortDownloads(list, "progress")[0]).toBe(b);
    expect(sortDownloads(list, "speed")[0]).toBe(a);
    expect(sortDownloads(list, "newest")[0]).toBe(d);
    expect(sortDownloads(list, "oldest")[0]).toBe(a);
  });

  it("counts", () => {
    expect(countDownloads(list)).toEqual({ all: 4, active: 2, completed: 1, failed: 1, torrent: 1, video: 1, running: 1 });
  });
});

import { reduceEvent } from "./downloads";
import { makeDownload } from "../test/fixtures";

describe("reduceEvent", () => {
  it("creates, updates and removes", () => {
    const d = makeDownload();
    let s = reduceEvent({}, { type: "download_created", download: d });
    expect(s[d.id]).toEqual(d);
    s = reduceEvent(s, { type: "download_completed", download: { ...d, status: "completed" } });
    expect(s[d.id].status).toBe("completed");
    s = reduceEvent(s, { type: "download_removed", id: d.id });
    expect(s).toEqual({});
  });

  it("applies batched progress with real values only", () => {
    const d = makeDownload({ downloadedBytes: 0 });
    const s0 = { [d.id]: d };
    const s1 = reduceEvent(s0, {
      type: "download_progress",
      updates: [
        { downloadId: d.id, status: "downloading", downloadedBytes: 600, totalBytes: 1000, speedBytesPerSecond: 300, uploadBytesPerSecond: 0, etaSeconds: 2, peers: null, seeds: null },
        { downloadId: "unknown", status: "downloading", downloadedBytes: 1, totalBytes: 2, speedBytesPerSecond: 1, uploadBytesPerSecond: 0, etaSeconds: 1, peers: null, seeds: null },
      ],
    });
    expect(s1[d.id]).toMatchObject({ downloadedBytes: 600, speedBps: 300, etaSeconds: 2 });
    expect(Object.keys(s1)).toEqual([d.id]);
    expect(s0[d.id].downloadedBytes).toBe(0); // immutable
  });

  it("ignores unrelated events without copying state", () => {
    const s = { a: makeDownload() };
    expect(reduceEvent(s, { type: "queue_finished" })).toBe(s);
    expect(reduceEvent(s, { type: "download_removed", id: "zzz" })).toBe(s);
  });
});

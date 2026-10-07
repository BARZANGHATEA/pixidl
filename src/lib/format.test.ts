import { formatBytes, formatEta, formatPercent, formatSpeed, parseLimitMBps, percent, limitToMBps, hostOf, formatDuration } from "./format";

describe("format", () => {
  it("formats bytes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(2.4 * 1024 ** 3)).toBe("2.4 GB");
    expect(formatBytes(null)).toBe("—");
    expect(formatBytes(-5)).toBe("—");
  });
  it("formats speed", () => {
    expect(formatSpeed(8.4 * 1024 * 1024)).toBe("8.4 MB/s");
    expect(formatSpeed(0)).toBe("0 B/s");
  });
  it("formats ETA like the reference design", () => {
    expect(formatEta(134)).toBe("2m 14s");
    expect(formatEta(62)).toBe("1m 02s");
    expect(formatEta(45)).toBe("45s");
    expect(formatEta(3700)).toBe("1h 1m");
    expect(formatEta(90000)).toBe("1d 1h");
    expect(formatEta(null)).toBeNull();
  });
  it("computes percentages only from real totals", () => {
    expect(percent(250, 1000)).toBe(25);
    expect(percent(5, null)).toBeNull();
    expect(percent(2000, 1000)).toBe(100);
    expect(formatPercent(63.9)).toBe("63%");
    expect(formatPercent(null)).toBe("");
  });
  it("parses speed limits", () => {
    expect(parseLimitMBps("10")).toBe(10 * 1024 * 1024);
    expect(parseLimitMBps("1,5")).toBe(1.5 * 1024 * 1024);
    expect(parseLimitMBps("")).toBeNull();
    expect(parseLimitMBps("0")).toBeNull();
    expect(limitToMBps(10 * 1024 * 1024)).toBe("10");
    expect(limitToMBps(null)).toBe("");
  });
  it("misc", () => {
    expect(hostOf("https://www.example.com/a")).toBe("www.example.com");
    expect(hostOf("magnet:?xt=urn:btih:abc")).toBe("magnet");
    expect(hostOf("nope")).toBe("");
    expect(formatDuration(3725)).toBe("1:02:05");
    expect(formatDuration(65)).toBe("1:05");
  });
});

// Pure helpers in src/lib.js. Run with: node --test browser-extension/test/
import { test } from "node:test";
import assert from "node:assert/strict";

import {
  DEFAULT_ACCENT,
  DEFAULT_SETTINGS,
  MAX_BATCH,
  MAX_DETECTED,
  PROTOCOL_VERSION,
  badgeText,
  basename,
  batchItems,
  buildEnvelope,
  buildItem,
  chunk,
  countByType,
  detectBrowser,
  displayName,
  extractUrlsFromText,
  fileExtension,
  fileTypeOf,
  filterItems,
  formatBytes,
  hostOf,
  isDownloadLink,
  isHttpUrl,
  meetsMinSize,
  normalizeEntries,
  normalizeLinks,
  normalizeResponse,
  percent,
  pingSummary,
  safeAccent,
  safeReferrer,
  sanitizeSettings,
  supportedUrl,
  totalSize,
  urlFileName,
  youtubeVideoUrl,
} from "../src/lib.js";

const MAGNET = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Linux%20ISO";

test("envelopes match protocol v1 and carry the client", () => {
  assert.deepEqual(buildEnvelope("ping", {}, "abc"), { version: PROTOCOL_VERSION, type: "ping", id: "abc", payload: {} });
  const env = buildEnvelope("probe_links", { items: [] }, "x", { browser: "brave", version: "2.0.0" });
  assert.deepEqual(env.client, { browser: "brave", version: "2.0.0" });
  assert.equal(buildEnvelope("ping", {}, "x", { browser: 1 }).client, undefined);
  const generated = buildEnvelope("get_status");
  assert.ok(typeof generated.id === "string" && generated.id.length > 0 && generated.id.length <= 128);
  assert.notEqual(generated.id, buildEnvelope("get_status").id);
});

test("responses are normalized", () => {
  assert.equal(normalizeResponse({ success: true, app_version: "1.0.0" }).success, true);
  assert.equal(normalizeResponse(undefined, "x").error.code, "internal");
  assert.equal(normalizeResponse([1]).error.code, "internal");
  assert.deepEqual(normalizeResponse({ success: false, error: { code: "not_found", message: "Download not found" } }).error, { code: "not_found", message: "Download not found" });
  assert.equal(normalizeResponse({ success: false }).error.code, "internal");
});

test("URL validation: only http(s) with a host and magnets with a hash", () => {
  assert.equal(supportedUrl(" https://example.com/a.zip#frag "), "https://example.com/a.zip");
  assert.equal(supportedUrl("http://example.com"), "http://example.com/");
  assert.equal(supportedUrl(MAGNET), MAGNET);
  assert.equal(supportedUrl(`${MAGNET}#x`), MAGNET);
  for (const bad of [
    "ftp://example.com/f",
    "file:///etc/passwd",
    "javascript:alert(1)",
    "data:text/plain,x",
    "blob:https://a.com/1",
    "magnet:?dn=nohash",
    "https://user:pw@a.com/x",
    "not a url",
    "",
    null,
    5,
    `https://a.com/${"x".repeat(17000)}`,
  ]) {
    assert.equal(supportedUrl(bad), null, String(bad).slice(0, 40));
  }
  assert.equal(isHttpUrl("https://a.com/x"), true);
  assert.equal(isHttpUrl(MAGNET), false);
  assert.equal(safeReferrer("javascript:alert(1)"), undefined);
  assert.equal(safeReferrer("https://a.com/page#top"), "https://a.com/page");
  assert.equal(hostOf("https://Sub.Example.com:8080/x"), "sub.example.com");
  assert.equal(hostOf(MAGNET), "");
  assert.equal(safeAccent("#10b981"), "#10b981");
  for (const bad of ["red", "#fff", "#12345g", "url(x)", null]) assert.equal(safeAccent(bad), DEFAULT_ACCENT);
});

test("link lists are validated and de-duplicated (fragment ignored), labels kept", () => {
  const links = normalizeEntries(
    [
      "https://a.com/1.zip",
      { url: "mailto:x@y.z", label: "mail" },
      { url: "https://a.com/1.zip#top", label: "  first\n zip " },
      "https://a.com/page",
      { url: "https://a.com/2.pdf", label: "PDF" },
      MAGNET,
      { url: "blob:https://a.com/x", label: "blob" },
    ],
    { exclude: ["https://a.com/page#section"] },
  );
  assert.deepEqual(links, [
    { url: "https://a.com/1.zip", label: "first zip" },
    { url: "https://a.com/2.pdf", label: "PDF" },
    { url: MAGNET, label: "" },
  ]);
  assert.equal(normalizeEntries(Array.from({ length: 50 }, (_, i) => `https://a.com/${i}`), { limit: 10 }).length, 10);
  assert.deepEqual(normalizeEntries(null), []);
  assert.deepEqual(normalizeLinks(["https://a.com/x#1", "https://a.com/x#2"]), ["https://a.com/x"]);
  assert.equal(normalizeEntries([{ url: "https://a.com/x", label: "y".repeat(500) }])[0].label.length, 200);
});

test("URLs typed in selected text are extracted", () => {
  const text = `Mirror: https://a.com/file.zip, or (see https://b.com/wiki/Foo_(bar)). Torrent ${MAGNET}.
    Persian: https://c.com/x.pdf، و ftp://d.com/no and www.e.com/yes "https://f.com/q?a=1&b=2"`;
  assert.deepEqual(extractUrlsFromText(text), [
    "https://a.com/file.zip",
    "https://b.com/wiki/Foo_(bar)",
    MAGNET,
    "https://c.com/x.pdf",
    "https://www.e.com/yes",
    "https://f.com/q?a=1&b=2",
  ]);
  assert.deepEqual(extractUrlsFromText(""), []);
  assert.deepEqual(extractUrlsFromText(null), []);
  assert.equal(extractUrlsFromText("https://a.com/1 https://a.com/2 https://a.com/3", 2).length, 2);
});

test("file names, extensions and types", () => {
  assert.equal(urlFileName("https://a.com/dir/My%20File.tar.gz?x=1"), "My File.tar.gz");
  assert.equal(urlFileName(MAGNET), "Linux ISO");
  assert.equal(fileExtension("https://a.com/x/Setup.EXE?dl=1"), "exe");
  assert.equal(fileExtension("archive.tar.gz"), "gz");
  assert.equal(fileExtension("https://a.com/dir/"), "");
  assert.equal(fileExtension(".hidden"), "");
  assert.equal(basename("C:\\Users\\me\\report.pdf"), "report.pdf");
  assert.equal(basename(undefined), "");

  const cases = [
    [{ url: "https://a.com/v.mkv" }, "video"],
    [{ url: "https://a.com/s.flac" }, "audio"],
    [{ url: "https://a.com/a.7z" }, "archive"],
    [{ url: "https://a.com/a.iso" }, "archive"],
    [{ url: "https://a.com/app.msi" }, "program"],
    [{ url: "https://a.com/app.AppImage" }, "program"],
    [{ url: "https://a.com/d.docx" }, "document"],
    [{ url: "https://a.com/p.webp" }, "image"],
    [{ url: "https://a.com/t.torrent" }, "torrent"],
    [{ url: MAGNET }, "torrent"],
    [{ url: "https://a.com/download?id=3" }, "other"],
    [{ url: "https://a.com/download?id=3", filename: "movie.mp4" }, "video"],
    [{ url: "https://a.com/download?id=3", contentType: "application/pdf; charset=binary" }, "document"],
    [{ url: "https://a.com/download?id=3", contentType: "application/x-bittorrent" }, "torrent"],
    [{ url: "https://a.com/get", contentType: "application/zip" }, "archive"],
    [{ url: "https://www.youtube.com/watch?v=x", engine: "video" }, "video"],
  ];
  for (const [input, expected] of cases) assert.equal(fileTypeOf(input), expected, JSON.stringify(input));
  assert.equal(displayName("https://a.com/x/file.zip", "label"), "file.zip");
  assert.equal(displayName("https://a.com/", "label"), "a.com");
});

test("download-link detection filter", () => {
  const anchor = (href, extra = {}) => isDownloadLink(href, { source: "anchor", ...extra });
  const media = (src) => isDownloadLink(src, { source: "media" });
  for (const url of [
    "https://a.com/f.zip",
    "https://a.com/f.ZIP?token=1",
    "https://a.com/f.tar.gz",
    "https://a.com/setup.exe",
    "https://a.com/app.AppImage",
    "https://a.com/book.epub",
    "https://a.com/data.csv",
    "https://a.com/clip.webm",
    "https://a.com/song.opus",
    "https://a.com/t.torrent",
    MAGNET,
    "https://a.com/photo.jpg",
  ]) {
    assert.equal(anchor(url), true, url);
  }
  for (const url of [
    "https://a.com/page.html",
    "https://a.com/readme.txt",
    "https://a.com/",
    "https://a.com/about",
    "javascript:void(0)",
    "data:application/zip,xx",
    "blob:https://a.com/1",
    "ftp://a.com/f.zip",
    "mailto:a@b.c",
  ]) {
    assert.equal(anchor(url), false, url);
  }
  // The download attribute makes any http(s) link count, never a bad scheme.
  assert.equal(anchor("https://a.com/export?id=3", { hasDownloadAttr: true }), true);
  assert.equal(anchor("blob:https://a.com/1", { hasDownloadAttr: true }), false);
  // Images only count when linked from an anchor, not as media sources.
  assert.equal(media("https://a.com/photo.png"), false);
  assert.equal(media("https://a.com/v/clip.mp4"), true);
  assert.equal(media("https://a.com/stream.m3u8"), false);
  assert.equal(MAX_DETECTED, 1000);
});

test("chunking and batching respect the protocol limits", () => {
  assert.deepEqual(chunk([1, 2, 3, 4, 5], 2), [[1, 2], [3, 4], [5]]);
  assert.deepEqual(chunk([], 50), []);
  assert.equal(chunk(Array.from({ length: 120 }), 50).length, 3);
  const items = Array.from({ length: 450 }, (_, i) => ({ url: `https://a.com/${i}` }));
  const batches = batchItems(items);
  assert.deepEqual(batches.map((b) => b.length), [MAX_BATCH, MAX_BATCH, 50]);
  assert.deepEqual(batches.flat(), items);
  const big = Array.from({ length: 10 }, (_, i) => ({ url: `https://a.com/${i}/${"x".repeat(1000)}` }));
  for (const batch of batchItems(big, 200, 3000)) assert.ok(JSON.stringify(batch).length <= 3000 || batch.length === 1);
  // The default byte budget (512 KiB) splits long URLs before the 200-item limit.
  const long = Array.from({ length: 200 }, (_, i) => ({ url: `https://a.com/${i}/${"y".repeat(4000)}` }));
  for (const batch of batchItems(long)) assert.ok(Buffer.byteLength(JSON.stringify(batch)) <= 512 * 1024);
  assert.ok(batchItems(long).length > 1);
  assert.deepEqual(batchItems([]), []);
});

test("protocol items carry only safe fields", () => {
  assert.deepEqual(buildItem("https://a.com/f.zip"), { url: "https://a.com/f.zip" });
  assert.deepEqual(buildItem("https://a.com/f.zip", { filename: "C:\\Users\\me\\Downloads\\f.zip", referrer: "https://a.com/" }), {
    url: "https://a.com/f.zip",
    filename: "f.zip",
    referrer: "https://a.com/",
  });
  assert.deepEqual(buildItem("https://a.com/f", { filename: "", referrer: "chrome://newtab/" }), { url: "https://a.com/f" });
  assert.equal(buildItem("https://youtu.be/x", { engine: "video" }).engine, "video");
  assert.equal(buildItem("https://a.com/x", { engine: "rm -rf" }).engine, undefined);
});

test("browser detection from user agents", () => {
  const chrome = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36";
  const cases = [
    [{ userAgent: chrome }, "chrome"],
    [{ userAgent: `${chrome} Edg/129.0.2792.52` }, "edge"],
    [{ userAgent: `${chrome} OPR/114.0.0.0` }, "opera"],
    [{ userAgent: `${chrome} Vivaldi/6.9.3447.48` }, "vivaldi"],
    [{ userAgent: chrome, isBrave: true }, "brave"],
    [{ userAgent: chrome, brands: [{ brand: "Brave", version: "129" }, { brand: "Chromium", version: "129" }] }, "brave"],
    [{ userAgent: chrome, brands: [{ brand: "Microsoft Edge", version: "129" }] }, "edge"],
    [{ userAgent: "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0" }, "firefox"],
    [{ userAgent: chrome, isFirefox: true }, "firefox"],
    [{}, "chrome"],
  ];
  for (const [input, expected] of cases) assert.equal(detectBrowser(input), expected, JSON.stringify(input));
});

test("formatting sizes, progress and the badge", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(formatBytes(5 * 1024 ** 3), "5.0 GB");
  assert.equal(formatBytes(null), "");
  assert.equal(formatBytes(-1), "");
  assert.equal(percent(50, 200), 25);
  assert.equal(percent(10, 0), null);
  assert.equal(percent(300, 200), 100);
  assert.equal(badgeText(0), "");
  assert.equal(badgeText(7), "7");
  assert.equal(badgeText(1000), "999+");
  assert.equal(badgeText("x"), "");
});

test("picker filtering by search words and type chips", () => {
  const items = [
    { url: "https://a.com/Movie.Trailer.mp4", name: "Movie.Trailer.mp4", type: "video", size: 100 },
    { url: "https://a.com/soundtrack.mp3", name: "soundtrack.mp3", type: "audio", size: null },
    { url: "https://cdn.b.org/setup.exe", name: "setup.exe", type: "program", size: 50 },
    { url: "https://a.com/manual.pdf", name: "User manual", label: "Docs", type: "document", size: 10 },
    { url: "https://a.com/clip.webm", name: "clip.webm", type: "video", size: 1 },
  ];
  const names = (list) => list.map((i) => i.name);
  assert.equal(filterItems(items).length, 5);
  assert.deepEqual(names(filterItems(items, { query: "movie" })), ["Movie.Trailer.mp4"]);
  assert.deepEqual(names(filterItems(items, { query: "b.org" })), ["setup.exe"]);
  assert.deepEqual(names(filterItems(items, { query: "a.com trailer" })), ["Movie.Trailer.mp4"]);
  assert.deepEqual(names(filterItems(items, { query: "docs" })), ["User manual"]);
  assert.deepEqual(names(filterItems(items, { types: ["video"] })), ["Movie.Trailer.mp4", "clip.webm"]);
  assert.deepEqual(names(filterItems(items, { types: new Set(["video", "audio"]), query: "sound" })), ["soundtrack.mp3"]);
  assert.deepEqual(filterItems(items, { types: ["torrent"] }), []);
  assert.deepEqual(countByType(items), { video: 2, audio: 1, program: 1, document: 1 });
  assert.deepEqual(Object.keys(countByType(items)), ["video", "audio", "program", "document"]);
  assert.deepEqual(totalSize(items), { bytes: 161, unknown: 1 });
});

test("YouTube canonical video URLs", () => {
  assert.equal(youtubeVideoUrl("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL123&index=2&t=42s"), "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
  assert.equal(youtubeVideoUrl("https://m.youtube.com/watch?feature=share&v=dQw4w9WgXcQ"), "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
  assert.equal(youtubeVideoUrl("https://www.youtube.com/shorts/abcDEF12345"), "https://www.youtube.com/watch?v=abcDEF12345");
  for (const bad of [
    "https://www.youtube.com/",
    "https://www.youtube.com/feed/subscriptions",
    "https://www.youtube.com/watch?list=PL1",
    "https://www.youtube.com/watch?v=<script>",
    "https://evil.com/watch?v=dQw4w9WgXcQ",
    "https://www.youtube.com.evil.com/watch?v=dQw4w9WgXcQ",
    "not a url",
  ]) {
    assert.equal(youtubeVideoUrl(bad), null, bad);
  }
});

test("settings and the cached ping", () => {
  // Detailed defaults, migration and capture rules: settings.test.mjs.
  assert.equal(DEFAULT_SETTINGS.captureDownloads, true);
  const s = sanitizeSettings({ settingsVersion: 2, selectionButton: false, detectLinks: "no", captureDownloads: "yes", minSizeKb: "12.7" });
  assert.equal(s.selectionButton, false);
  assert.equal(s.detectLinks, true);
  assert.equal(s.captureDownloads, true, "a non-boolean falls back to the default");
  assert.equal(s.minSizeKb, 12);
  assert.equal(sanitizeSettings({ settingsVersion: 2, minSizeKb: "-3" }).minSizeKb, 0);
  assert.equal(meetsMinSize(5 * 1024 * 1024, 10 * 1024), false);
  assert.equal(meetsMinSize(20 * 1024 * 1024, 10 * 1024), true);
  assert.equal(meetsMinSize(-1, 10), true, "unknown size is captured");

  const ok = pingSummary({ success: true, app_version: "1.0.0", accent_color: "#10B981", language: "fa", integration_enabled: true }, 5);
  assert.deepEqual(ok, { connected: true, app_version: "1.0.0", accent_color: "#10B981", language: "fa", integration_enabled: true, checked_at: 5 });
  assert.equal(pingSummary({ success: true, accent_color: "javascript:x" }).accent_color, DEFAULT_ACCENT);
  assert.deepEqual(pingSummary({ success: false, error: { code: "app_unavailable" } }, 7), { connected: false, error_code: "app_unavailable", checked_at: 7 });
});

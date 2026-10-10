// Settings defaults and migration, URLs in selected text, and the rules that
// decide which browser downloads go to pixidl. Run with: node --test browser-extension/test/
import { test } from "node:test";
import assert from "node:assert/strict";

import {
  BYPASS_TTL_MS,
  DEFAULT_SETTINGS,
  SETTINGS_STORAGE_KEYS,
  SETTINGS_VERSION,
  UNREACHABLE_GRACE_MS,
  appReachable,
  captureDecision,
  extractUrlsFromText,
  hostMatches,
  migrateSettings,
  normalizeHost,
  parseExtensionList,
  sanitizeSettings,
} from "../src/lib.js";

const MAGNET = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Linux%20ISO";
const NOW = 1_800_000_000_000;

// ---- URLs in selected text ------------------------------------------------------

test("selection text: addresses typed in prose, forums and search results", () => {
  const forum = `Download links:
    Part 1: https://files.example.com/a.part1.rar
    Part 2: https://files.example.com/a.part2.rar
    Mirror (slow): http://mirror.example.org/a.zip.`;
  assert.deepEqual(extractUrlsFromText(forum), [
    "https://files.example.com/a.part1.rar",
    "https://files.example.com/a.part2.rar",
    "http://mirror.example.org/a.zip",
  ]);
  // A search result snippet shows the address without a scheme.
  assert.deepEqual(extractUrlsFromText("Result — www.example.com/downloads/tool.msi · 2 MB"), ["https://www.example.com/downloads/tool.msi"]);
  assert.deepEqual(extractUrlsFromText(`torrent: ${MAGNET}`), [MAGNET]);
  // Quotes, brackets and sentence punctuation around an address are not part of it.
  assert.deepEqual(extractUrlsFromText(`«https://a.com/x.zip» [https://b.com/y.iso] "https://c.com/z.7z"!`), [
    "https://a.com/x.zip",
    "https://b.com/y.iso",
    "https://c.com/z.7z",
  ]);
  // Persian text with Persian punctuation right after the address.
  assert.deepEqual(extractUrlsFromText("فایل را از https://example.com/fa/setup.exe، دریافت کنید."), ["https://example.com/fa/setup.exe"]);
});

test("selection text: what is not an address", () => {
  assert.deepEqual(extractUrlsFromText("This paragraph has no addresses at all, only ordinary words."), []);
  assert.deepEqual(extractUrlsFromText("e-mail me@www.example.com or see foo.www.bar.com"), []);
  assert.deepEqual(extractUrlsFromText("ftp://ftp.example.com/pub/tool.tar.gz file:///C:/x javascript:alert(1)"), [], "pixidl takes only http(s) and magnet");
  assert.deepEqual(extractUrlsFromText("magnet:?dn=no-hash https:// www. www.x"), []);
  assert.deepEqual(extractUrlsFromText("https://user:pass@example.com/secret.zip"), [], "addresses with credentials are dropped");
});

// ---- Defaults and migration -------------------------------------------------------

test("defaults: capture is on, 100 KB minimum, selection button on", () => {
  const fresh = sanitizeSettings({});
  assert.deepEqual(fresh, {
    selectionButton: true,
    detectLinks: true,
    showBadge: true,
    youtubeButton: true,
    captureDownloads: true,
    minSizeKb: 100,
    captureNotice: true,
    captureExcludedSites: [],
    captureSkipExtensions: [],
  });
  assert.deepEqual(sanitizeSettings(undefined), fresh);
  assert.deepEqual(sanitizeSettings({ settingsVersion: SETTINGS_VERSION }), fresh);
  assert.equal(DEFAULT_SETTINGS.captureDownloads, true);
  for (const key of [...Object.keys(DEFAULT_SETTINGS), "settingsVersion", "minSizeMb"]) assert.ok(SETTINGS_STORAGE_KEYS.includes(key), key);
});

test("migration from 2.0 turns capture on once and converts the size to KB", () => {
  // 2.0 stored every option as soon as one was changed.
  const v1 = { selectionButton: false, detectLinks: true, showBadge: true, youtubeButton: true, captureDownloads: false, minSizeMb: 0 };
  const plan = migrateSettings(v1);
  assert.deepEqual(plan.remove, ["minSizeMb"]);
  assert.equal(plan.set.settingsVersion, SETTINGS_VERSION);
  assert.equal(plan.set.captureDownloads, true);
  assert.equal(plan.set.selectionButton, false, "other choices are kept");
  assert.equal(plan.set.minSizeKb, 100, "the old default (0 MB) takes the new default");
  assert.equal(migrateSettings({ ...v1, minSizeMb: 5 }).set.minSizeKb, 5 * 1024, "a chosen minimum is kept");
  // Fresh install: nothing stored yet.
  assert.equal(migrateSettings({}).set.captureDownloads, true);
  // Already migrated: nothing to do, and the user's choice sticks.
  const v2 = { ...plan.set, captureDownloads: false };
  assert.equal(migrateSettings(v2), null);
  assert.equal(sanitizeSettings(v2).captureDownloads, false);
  assert.equal(sanitizeSettings({ ...v2, minSizeKb: 0 }).minSizeKb, 0);
  // A stray legacy key is cleaned up.
  assert.deepEqual(migrateSettings({ ...v2, minSizeMb: 3 }).remove, ["minSizeMb"]);
  assert.equal(migrateSettings({ ...v2, minSizeMb: 3 }).set.captureDownloads, false);
});

test("site and file-type lists are cleaned", () => {
  const s = sanitizeSettings({
    settingsVersion: 2,
    captureExcludedSites: ["https://www.Example.com/page", "drive.google.com", "", "bad host!", 5, "drive.google.com"],
    captureSkipExtensions: " .JPG, png;*.pdf  toolongextension, png",
  });
  assert.deepEqual(s.captureExcludedSites, ["example.com", "drive.google.com"]);
  assert.deepEqual(s.captureSkipExtensions, ["jpg", "png", "pdf"]);
  assert.equal(normalizeHost("www.github.com"), "github.com");
  assert.equal(normalizeHost("https://a.b.example.org:8080/x"), "a.b.example.org");
  assert.equal(normalizeHost(""), null);
  assert.ok(hostMatches("dl.example.com", ["example.com"]));
  assert.ok(hostMatches("example.com", ["example.com"]));
  assert.ok(hostMatches("www.example.com", ["example.com"]));
  assert.ok(!hostMatches("badexample.com", ["example.com"]));
  assert.ok(!hostMatches("", ["example.com"]));
  assert.deepEqual(parseExtensionList(["ZIP", ".iso", "x y"]), ["zip", "iso"]);
});

// ---- Which downloads are captured ---------------------------------------------------

const settings = (patch = {}) => ({ ...sanitizeSettings({ settingsVersion: 2 }), ...patch });
const download = (patch = {}) => ({
  id: 7,
  url: "https://cdn.example.com/files/app.zip",
  finalUrl: "https://cdn.example.com/files/app.zip",
  referrer: "https://example.com/downloads",
  filename: "app.zip",
  mime: "application/zip",
  totalBytes: 5 * 1024 * 1024,
  fileSize: 5 * 1024 * 1024,
  incognito: false,
  state: "in_progress",
  ...patch,
});
const connected = { connected: true, integration_enabled: true, checked_at: NOW - 60_000 };
const decide = (item, ctx = {}) => captureDecision(item, { settings: settings(), ping: connected, now: NOW, ...ctx });

test("capture: an ordinary download goes to pixidl, with the final URL", () => {
  assert.deepEqual(decide(download()), { capture: true, reason: "ok", url: "https://cdn.example.com/files/app.zip" });
  const redirected = decide(download({ url: "https://example.com/get?id=1", finalUrl: "https://cdn.example.com/x/app.zip#frag" }));
  assert.equal(redirected.url, "https://cdn.example.com/x/app.zip");
  // Unknown size (no Content-Length) is captured.
  assert.equal(decide(download({ totalBytes: -1, fileSize: 0 })).capture, true);
  // No ping yet (first run) or an old failure: try, the request decides.
  assert.equal(decide(download(), { ping: null }).capture, true);
  assert.equal(decide(download(), { ping: { connected: false, checked_at: NOW - UNREACHABLE_GRACE_MS - 1 } }).capture, true);
});

test("capture: what stays in the browser", () => {
  const reason = (item, ctx) => decide(item, ctx).reason;
  assert.equal(reason(download(), { settings: settings({ captureDownloads: false }) }), "disabled");
  assert.equal(reason(download({ incognito: true })), "incognito");
  assert.equal(reason(download({ state: "complete" })), "not_in_progress");
  assert.equal(reason(download({ byExtensionId: "abcdefghijklmnopabcdefghijklmnop" })), "by_extension");
  for (const url of ["blob:https://example.com/0f1e", "data:application/octet-stream;base64,AAAA", "file:///C:/x.zip", "filesystem:https://a.com/temporary/x", "chrome-extension://abc/x.zip"]) {
    assert.equal(reason(download({ url, finalUrl: url })), "browser_only_scheme", url);
  }
  assert.equal(reason(download({ url: "ftp://ftp.example.com/x.zip", finalUrl: "ftp://ftp.example.com/x.zip" })), "unsupported_url");
  assert.equal(reason(download({ totalBytes: 50 * 1024 })), "too_small");
  assert.equal(reason(download({ totalBytes: 50 * 1024 }), { settings: settings({ minSizeKb: 0 }) }), "ok");
  assert.equal(reason(download(), { settings: settings({ captureSkipExtensions: ["zip"] }) }), "skipped_type");
  assert.equal(reason(download({ filename: "C:\\Users\\me\\Downloads\\photo.JPG" }), { settings: settings({ captureSkipExtensions: ["jpg"] }) }), "skipped_type");
  // Per-site exclusion matches the page (referrer) or the file's host.
  assert.equal(reason(download(), { settings: settings({ captureExcludedSites: ["example.com"] }) }), "excluded_site");
  assert.equal(reason(download({ referrer: "https://other.org/" }), { settings: settings({ captureExcludedSites: ["cdn.example.com"] }) }), "excluded_site");
  assert.equal(reason(download({ referrer: "https://other.org/" }), { settings: settings({ captureExcludedSites: ["another.net"] }) }), "ok");
  // pixidl failed to answer a moment ago, or browser integration is off in the app.
  assert.equal(reason(download(), { ping: { connected: false, error_code: "app_unavailable", checked_at: NOW - 5_000 } }), "app_unreachable");
  assert.equal(reason(download(), { ping: { connected: true, integration_enabled: false, checked_at: NOW } }), "app_unreachable");
});

test("capture: Alt+click bypass and the extension's own restarted downloads", () => {
  const item = download({ url: "https://example.com/files/app.zip", finalUrl: "https://cdn.example.com/app.zip" });
  assert.equal(decide(item, { bypass: [{ url: "https://example.com/files/app.zip", until: NOW + BYPASS_TTL_MS }] }).reason, "bypass");
  assert.equal(decide(item, { bypass: [{ url: "https://example.com/files/app.zip", until: NOW - 1 }] }).reason, "ok", "an expired bypass no longer applies");
  assert.equal(decide(item, { bypass: [{ url: "https://example.com/other.zip", until: NOW + 1000 }] }).reason, "ok");
  assert.equal(decide(item, { ownUrls: ["https://example.com/files/app.zip#x"] }).reason, "own_download");
  assert.equal(decide(null).reason, "invalid");
});

test("reachability from the cached ping", () => {
  assert.equal(appReachable(null, NOW), true);
  assert.equal(appReachable({ connected: true, checked_at: 0 }, NOW), true);
  assert.equal(appReachable({ connected: false, checked_at: NOW - 1000 }, NOW), false);
  assert.equal(appReachable({ connected: false, checked_at: NOW - UNREACHABLE_GRACE_MS }, NOW), true);
  assert.equal(appReachable({ connected: false }, NOW), true, "no time recorded: try");
});

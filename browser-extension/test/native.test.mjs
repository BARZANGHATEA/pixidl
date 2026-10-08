// Run with: node --test browser-extension/test/
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  MAX_BATCH,
  PROTOCOL_VERSION,
  basename,
  batchItems,
  buildEnvelope,
  buildItem,
  formatBytes,
  isHttpUrl,
  matchesExtensions,
  meetsMinSize,
  normalizeLinks,
  normalizeResponse,
  parseExtensionFilter,
  percent,
  safeReferrer,
  sanitizeSettings,
  supportedUrl,
} from "../lib.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const readJson = (path) => JSON.parse(readFileSync(join(root, path), "utf8"));
const MAGNET = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=x";

test("envelope matches protocol v1", () => {
  const env = buildEnvelope("ping", {}, "abc");
  assert.deepEqual(env, { version: PROTOCOL_VERSION, type: "ping", id: "abc", payload: {} });
  const generated = buildEnvelope("get_status");
  assert.equal(typeof generated.id, "string");
  assert.ok(generated.id.length > 0 && generated.id.length <= 128);
  assert.notEqual(generated.id, buildEnvelope("get_status").id);
});

test("responses are normalized", () => {
  assert.equal(normalizeResponse({ success: true, app_version: "1.0.0" }).success, true);
  assert.deepEqual(normalizeResponse(undefined, "x").error.code, "internal");
  assert.deepEqual(normalizeResponse([1]).error.code, "internal");
  const err = normalizeResponse({ success: false, error: { code: "not_found", message: "Download not found" } });
  assert.deepEqual(err.error, { code: "not_found", message: "Download not found" });
  assert.equal(normalizeResponse({ success: false }).error.code, "internal");
});

test("only http(s) and magnet URLs are accepted", () => {
  assert.equal(supportedUrl(" https://example.com/a.zip#frag "), "https://example.com/a.zip");
  assert.equal(supportedUrl("http://example.com"), "http://example.com/");
  assert.equal(supportedUrl(MAGNET), MAGNET);
  for (const bad of ["ftp://example.com/f", "file:///etc/passwd", "javascript:alert(1)", "data:text/plain,x", "blob:https://a.com/1", "magnet:?dn=nohash", "not a url", "", null, 5, `https://a.com/${"x".repeat(17000)}`]) {
    assert.equal(supportedUrl(bad), null, String(bad).slice(0, 40));
  }
  assert.equal(isHttpUrl("https://a.com/x"), true);
  assert.equal(isHttpUrl(MAGNET), false);
  assert.equal(safeReferrer("javascript:alert(1)"), undefined);
  assert.equal(safeReferrer("https://a.com/page"), "https://a.com/page");
});

test("collected links are validated and de-duplicated in order", () => {
  const links = normalizeLinks(
    ["https://a.com/1.zip", "mailto:x@y.z", "https://a.com/1.zip#top", "https://a.com/page", "https://a.com/2.pdf", MAGNET, "https://a.com/1.zip"],
    { exclude: ["https://a.com/page#section"] },
  );
  assert.deepEqual(links, ["https://a.com/1.zip", "https://a.com/2.pdf", MAGNET]);
  assert.equal(normalizeLinks(Array.from({ length: 50 }, (_, i) => `https://a.com/${i}`), { limit: 10 }).length, 10);
  assert.deepEqual(normalizeLinks(null), []);
});

test("extension filter parsing and matching", () => {
  assert.deepEqual(parseExtensionFilter("zip, .PDF *.tar.gz;mp4 zip"), ["zip", "pdf", "tar.gz", "mp4"]);
  assert.deepEqual(parseExtensionFilter("  "), []);
  assert.deepEqual(parseExtensionFilter("../x, <b>"), []);
  const exts = parseExtensionFilter("zip,tar.gz");
  assert.equal(matchesExtensions("https://a.com/files/Archive.ZIP?dl=1", exts), true);
  assert.equal(matchesExtensions("https://a.com/b.tar.gz", exts), true);
  assert.equal(matchesExtensions("https://a.com/zip/readme", exts), false);
  assert.equal(matchesExtensions(MAGNET, exts), false);
  assert.equal(matchesExtensions("https://a.com/anything", []), true);
});

test("batches respect the 200-item and byte limits", () => {
  const items = Array.from({ length: 450 }, (_, i) => ({ url: `https://a.com/${i}` }));
  const batches = batchItems(items);
  assert.deepEqual(batches.map((b) => b.length), [MAX_BATCH, MAX_BATCH, 50]);
  assert.deepEqual(batches.flat(), items);
  const big = Array.from({ length: 10 }, (_, i) => ({ url: `https://a.com/${i}/${"x".repeat(1000)}` }));
  for (const batch of batchItems(big, 200, 3000)) assert.ok(JSON.stringify(batch).length <= 3000 || batch.length === 1);
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
  assert.equal(basename("/home/me/Downloads/report.pdf"), "report.pdf");
  assert.equal(basename(undefined), "");
});

test("progress, sizes and options", () => {
  assert.equal(percent(50, 200), 25);
  assert.equal(percent(10, 0), null);
  assert.equal(percent(10, null), null);
  assert.equal(percent(300, 200), 100);
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(meetsMinSize(5 * 1024 * 1024, 10), false);
  assert.equal(meetsMinSize(20 * 1024 * 1024, 10), true);
  assert.equal(meetsMinSize(-1, 10), true, "unknown size is captured");
  assert.equal(meetsMinSize(100, 0), true);
  assert.deepEqual(sanitizeSettings(undefined), { captureDownloads: false, minSizeMb: 0 });
  assert.deepEqual(sanitizeSettings({ captureDownloads: "yes", minSizeMb: "-3" }), { captureDownloads: false, minSizeMb: 0 });
  assert.deepEqual(sanitizeSettings({ captureDownloads: true, minSizeMb: "12.7" }), { captureDownloads: true, minSizeMb: 12 });
});

test("manifest is valid and complete", () => {
  const m = readJson("manifest.json");
  assert.equal(m.manifest_version, 3);
  assert.equal(m.default_locale, "en");
  assert.equal(m.browser_specific_settings.gecko.id, "pixidl@pixidl.app");
  assert.equal(m.background.service_worker, "background.js");
  assert.deepEqual(m.background.scripts, ["background.js"]);
  assert.ok(typeof m.key === "string" && m.key.length > 300);
  for (const p of ["nativeMessaging", "contextMenus", "storage", "activeTab", "scripting", "notifications", "downloads"]) {
    assert.ok(m.permissions.includes(p), p);
  }
  assert.equal(m.host_permissions, undefined);
  for (const icon of Object.values(m.icons)) assert.ok(readFileSync(join(root, icon)).length > 0);
});

test("locales have identical keys and cover every key used in code", () => {
  const en = readJson("_locales/en/messages.json");
  const fa = readJson("_locales/fa/messages.json");
  assert.deepEqual(Object.keys(fa).sort(), Object.keys(en).sort());
  for (const [key, value] of Object.entries(en)) {
    assert.deepEqual(Object.keys(fa[key].placeholders ?? {}).sort(), Object.keys(value.placeholders ?? {}).sort(), key);
  }
  const used = new Set();
  for (const file of readdirSync(root).filter((f) => /\.(js|html|json)$/.test(f))) {
    const src = readFileSync(join(root, file), "utf8");
    for (const [, key] of src.matchAll(/(?:\bt\(|getMessage\()"([A-Za-z_]+)"/g)) used.add(key);
    for (const [, key] of src.matchAll(/data-i18n(?:-placeholder)?="([A-Za-z_]+)"/g)) used.add(key);
    for (const [, key] of src.matchAll(/__MSG_([A-Za-z_]+)__/g)) used.add(key);
  }
  for (const action of ["pause", "resume", "cancel"]) used.add(action);
  assert.ok(used.size > 20);
  for (const key of used) assert.ok(key in en, `missing locale key: ${key}`);
});

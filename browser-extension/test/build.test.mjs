// Packaging, manifests, locales and syntax. Run with: node --test browser-extension/test/
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, readdirSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import zlib from "node:zlib";

import { FIREFOX_ID, build, bundleContentScript, createZip, generateManifest } from "../build.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const src = join(root, "src");
const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));
const base = readJson(join(root, "manifest.base.json"));
const CHROMIUM_ID = "ndlafmjbcbcjmkegfelbhgknmajgdbna";

/** Minimal ZIP reader: central directory -> {name: Buffer}. */
function readZip(buffer) {
  const eocd = buffer.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  assert.ok(eocd >= 0, "end of central directory");
  const count = buffer.readUInt16LE(eocd + 10);
  let p = buffer.readUInt32LE(eocd + 16);
  const files = {};
  for (let i = 0; i < count; i++) {
    assert.equal(buffer.readUInt32LE(p), 0x02014b50, "central header");
    const method = buffer.readUInt16LE(p + 10);
    const crc = buffer.readUInt32LE(p + 16);
    const compressed = buffer.readUInt32LE(p + 20);
    const size = buffer.readUInt32LE(p + 24);
    const nameLen = buffer.readUInt16LE(p + 28);
    const extraLen = buffer.readUInt16LE(p + 30);
    const commentLen = buffer.readUInt16LE(p + 32);
    const offset = buffer.readUInt32LE(p + 42);
    const name = buffer.toString("utf8", p + 46, p + 46 + nameLen);
    assert.equal(buffer.readUInt32LE(offset), 0x04034b50, `local header of ${name}`);
    const start = offset + 30 + buffer.readUInt16LE(offset + 26) + buffer.readUInt16LE(offset + 28);
    const body = buffer.subarray(start, start + compressed);
    const data = method === 8 ? zlib.inflateRawSync(body) : Buffer.from(body);
    assert.equal(data.length, size, `size of ${name}`);
    assert.equal(zlib.crc32(data) >>> 0, crc, `crc of ${name}`);
    files[name] = data;
    p += 46 + nameLen + extraLen + commentLen;
  }
  return files;
}

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    return statSync(full).isDirectory() ? walk(full) : [full];
  });
}

test("the Chromium key gives the reference extension ID", () => {
  const hash = createHash("sha256").update(Buffer.from(base.key, "base64")).digest("hex").slice(0, 32);
  const id = [...hash].map((c) => String.fromCharCode(97 + parseInt(c, 16))).join("");
  assert.equal(id, CHROMIUM_ID);
});

test("manifests per browser", () => {
  const chromium = generateManifest(base, "chromium");
  assert.deepEqual(chromium.background, { service_worker: "background.js", type: "module" });
  assert.equal(chromium.key, base.key);
  assert.equal(chromium.browser_specific_settings, undefined);

  const firefox = generateManifest(base, "firefox");
  assert.deepEqual(firefox.background, { scripts: ["background.js"], type: "module" });
  assert.equal(firefox.key, undefined);
  assert.equal(firefox.minimum_chrome_version, undefined);
  assert.deepEqual(firefox.browser_specific_settings.gecko, {
    id: FIREFOX_ID,
    strict_min_version: "128.0",
    data_collection_permissions: { required: ["none"] },
  });

  for (const m of [chromium, firefox]) {
    assert.equal(m.manifest_version, 3);
    assert.equal(m.version, "2.0.0");
    assert.equal(m.default_locale, "en");
    assert.equal(m.name, "__MSG_extName__");
    for (const p of ["nativeMessaging", "contextMenus", "storage", "notifications", "downloads", "scripting", "activeTab", "tabs", "alarms"]) {
      assert.ok(m.permissions.includes(p), p);
    }
    assert.deepEqual(m.host_permissions, ["<all_urls>"]);
    assert.deepEqual(m.content_scripts, [{ matches: ["<all_urls>"], js: ["content.js"], run_at: "document_idle", all_frames: false }]);
    assert.deepEqual(m.options_ui, { page: "options.html", open_in_tab: false });
  }
  assert.throws(() => generateManifest(base, "safari"));
  // The base manifest is not modified.
  assert.equal(base.background, undefined);
});

test("the content script bundle is one classic script", () => {
  const lib = readFileSync(join(src, "lib.js"), "utf8");
  const bundle = bundleContentScript(lib, readFileSync(join(src, "content.js"), "utf8"));
  assert.doesNotThrow(() => new vm.Script(bundle, { filename: "content.js" }));
  assert.ok(!/^\s*(import|export)\b/m.test(bundle));
  assert.ok(bundle.includes("function supportedUrl("));
  // Running it without extension APIs does nothing (and does not throw).
  assert.doesNotThrow(() => vm.runInNewContext(bundle, { window: {}, globalThis: {} }));
  assert.throws(() => bundleContentScript('import x from "./y.js";', ""));
  assert.throws(() => bundleContentScript("export default 1;", ""));
  assert.throws(() => bundleContentScript("", 'import "./lib.js";'));
});

test("the ZIP writer round-trips", () => {
  const entries = [
    { name: "manifest.json", data: Buffer.from('{"a":1}') },
    { name: "dir/sub/file.txt", data: Buffer.from("hello ".repeat(500)) },
    { name: "empty.bin", data: Buffer.alloc(0) },
    { name: "fa/متن.txt", data: Buffer.from("سلام") },
  ];
  const files = readZip(createZip(entries));
  assert.deepEqual(Object.keys(files), entries.map((e) => e.name));
  for (const e of entries) assert.deepEqual(files[e.name], e.data);
  assert.throws(() => createZip([{ name: "..\\evil", data: Buffer.alloc(1) }]));
  assert.throws(() => createZip([{ name: "/abs", data: Buffer.alloc(1) }]));
});

test("build.mjs produces both packages, deterministically", () => {
  const out1 = mkdtempSync(join(tmpdir(), "pixidl-ext-"));
  const out2 = mkdtempSync(join(tmpdir(), "pixidl-ext-"));
  try {
    const first = build({ root, outDir: out1 });
    const second = build({ root, outDir: out2 });
    for (const target of ["chromium", "firefox"]) {
      const archive = readFileSync(first[target].archive);
      assert.deepEqual(archive, readFileSync(second[target].archive), `${target} build is deterministic`);
      const files = readZip(archive);
      const names = Object.keys(files);
      assert.ok(names.includes("manifest.json"), "manifest.json at the root");
      assert.ok(names.every((n) => !n.includes("\\") && !n.startsWith("/")), "forward-slash paths");
      assert.deepEqual(names, [...names].sort());

      const manifest = JSON.parse(files["manifest.json"].toString("utf8"));
      assert.deepEqual(manifest, readJson(join(first[target].dir, "manifest.json")));
      if (target === "chromium") {
        assert.equal(manifest.background.service_worker, "background.js");
        assert.equal(manifest.background.scripts, undefined);
        assert.equal(manifest.key, base.key);
        assert.equal(manifest.browser_specific_settings, undefined);
      } else {
        assert.deepEqual(manifest.background.scripts, ["background.js"]);
        assert.equal(manifest.background.service_worker, undefined);
        assert.equal(manifest.key, undefined);
        assert.equal(manifest.browser_specific_settings.gecko.id, FIREFOX_ID);
      }

      // Everything the manifest and the pages reference is in the package.
      const referenced = [
        manifest.action.default_popup,
        manifest.options_ui.page,
        ...manifest.content_scripts.flatMap((c) => c.js),
        ...(manifest.background.scripts ?? [manifest.background.service_worker]),
        ...Object.values(manifest.icons),
        ...Object.values(manifest.action.default_icon),
        "picker.html",
        "_locales/en/messages.json",
        "_locales/fa/messages.json",
      ];
      for (const page of names.filter((n) => n.endsWith(".html"))) {
        const html = files[page].toString("utf8");
        for (const [, ref] of html.matchAll(/(?:src|href)="([^"#:]+)"/g)) referenced.push(ref);
      }
      for (const page of names.filter((n) => n.endsWith(".js") && n !== "content.js")) {
        for (const [, ref] of files[page].toString("utf8").matchAll(/from "\.\/([^"]+)"/g)) referenced.push(ref);
      }
      for (const ref of referenced) assert.ok(names.includes(ref), `${target}: missing ${ref}`);
      assert.ok(!names.includes("lib.test.mjs") && names.every((n) => !n.startsWith(".")));
      assert.ok(files["content.js"].toString("utf8").includes("function isDownloadLink("), "content.js is bundled with lib.js");
    }
  } finally {
    rmSync(out1, { recursive: true, force: true });
    rmSync(out2, { recursive: true, force: true });
  }
});

test("every JavaScript file passes node --check", () => {
  const files = [...walk(src).filter((f) => f.endsWith(".js")), join(root, "build.mjs")];
  assert.ok(files.length >= 10);
  for (const file of files) execFileSync(process.execPath, ["--check", file], { stdio: "pipe" });
});

test("locales have identical keys and cover every key used", () => {
  const en = readJson(join(src, "_locales/en/messages.json"));
  const fa = readJson(join(src, "_locales/fa/messages.json"));
  assert.deepEqual(Object.keys(fa).sort(), Object.keys(en).sort());
  for (const [key, value] of Object.entries(en)) {
    assert.deepEqual(Object.keys(fa[key].placeholders ?? {}).sort(), Object.keys(value.placeholders ?? {}).sort(), key);
    for (const name of Object.keys(value.placeholders ?? {})) {
      for (const [lang, messages] of [["en", en], ["fa", fa]]) {
        assert.ok(messages[key].message.includes(`$${name.toUpperCase()}$`), `${lang}.${key} uses $${name}$`);
      }
    }
    assert.ok(fa[key].message.trim().length > 0, key);
  }
  assert.equal(en.uiDirection.message, "ltr");
  assert.equal(fa.uiDirection.message, "rtl");

  const used = new Set();
  const sources = [...walk(src).filter((f) => /\.(js|html)$/.test(f)), join(root, "manifest.base.json")];
  for (const file of sources) {
    const text = readFileSync(file, "utf8");
    for (const [, key] of text.matchAll(/(?:\bt\(|getMessage\()"([A-Za-z_]+)"/g)) used.add(key);
    for (const [, key] of text.matchAll(/data-i18n(?:-placeholder|-title|-aria)?="([A-Za-z_]+)"/g)) used.add(key);
    for (const [, key] of text.matchAll(/__MSG_([A-Za-z_]+)__/g)) used.add(key);
  }
  for (const action of ["pause", "resume", "cancel"]) used.add(action);
  for (const status of ["queued", "preparing", "downloading", "paused", "completed", "failed", "cancelled"]) used.add(`status_${status}`);
  for (const type of ["video", "audio", "archive", "program", "document", "image", "torrent", "other"]) used.add(`type_${type}`);
  assert.ok(used.size > 60);
  for (const key of used) assert.ok(key in en, `missing locale key: ${key}`);
  const unused = Object.keys(en).filter((key) => !used.has(key));
  assert.deepEqual(unused, [], "unused locale keys");
});

test("the product name is always lowercase", () => {
  for (const file of walk(src).filter((f) => /\.(js|html|json|css)$/.test(f))) {
    const text = readFileSync(file, "utf8");
    assert.ok(!/Pixidl|PIXIDL|PixiDL/.test(text), file);
  }
});

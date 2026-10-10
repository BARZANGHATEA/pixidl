#!/usr/bin/env node
// End-to-end test of the Chromium build in a real, headed Chromium driven by
// Playwright. Not part of `node --test` (it needs Playwright, a Chromium and a
// display); run it by hand:
//
//   node browser-extension/build.mjs
//   DISPLAY=:99 node browser-extension/e2e/run.mjs [--out <screenshot dir>]
//
// Environment: PLAYWRIGHT_MODULES (folder holding the `playwright` package,
// default /opt/node22/lib/node_modules/), CHROMIUM (browser binary, default
// /opt/pw-browsers/chromium).
//
// It serves e2e/site/ over http://127.0.0.1, registers e2e/fake-host.py as the
// com.pixidl.app native host inside the test profile, loads dist/chromium
// unpacked and checks with real mouse and keyboard input:
// - the selection button (links, typed addresses, double/triple-click,
//   keyboard, text fields, Escape, scrolling, zoom, RTL, a missed pointerup);
// - that tabs opened before the extension was reloaded or re-enabled get the
//   content script without a page reload;
// - download capture: sent to the host and removed from the browser, and every
//   fallback (host error, app unavailable, broken host, small file, blob:,
//   Alt+click, excluded site, capture off) leaving the download to the browser.

import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import os from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const EXT_ID = "ndlafmjbcbcjmkegfelbhgknmajgdbna";
const args = process.argv.slice(2);
const arg = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? resolve(args[i + 1]) : fallback;
};
const EXT_DIR = arg("--ext", join(HERE, "..", "dist", "chromium"));
const OUT_DIR = arg("--out", join(os.tmpdir(), "pixidl-e2e-shots"));
const require = createRequire(process.env.PLAYWRIGHT_MODULES || "/opt/node22/lib/node_modules/");
const { chromium } = require("playwright");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const results = [];
function check(name, ok, detail = "") {
  results.push({ name, ok: Boolean(ok), detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  (${detail})` : ""}`);
}

async function waitFor(fn, { timeout = 8000, interval = 100 } = {}) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    last = await fn();
    if (last) return last;
    await sleep(interval);
  }
  return last;
}

function freePort() {
  return new Promise((ok, fail) => {
    const srv = createServer();
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => ok(port));
    });
    srv.on("error", fail);
  });
}

// ---- Test environment ------------------------------------------------------------------

const tmp = mkdtempSync(join(os.tmpdir(), "pixidl-e2e-"));
const www = join(tmp, "www");
const hostDir = join(tmp, "host");
const profile = join(tmp, "profile");
const downloadDir = join(tmp, "downloads");
mkdirSync(OUT_DIR, { recursive: true });
for (const dir of [hostDir, profile, downloadDir]) mkdirSync(dir, { recursive: true });
cpSync(join(HERE, "site"), www, { recursive: true });
mkdirSync(join(www, "files"), { recursive: true });
const bytes = (n, seed) => Buffer.alloc(n, seed);
for (const [name, size] of [
  ["app.zip", 3 << 20],
  ["second.zip", 3 << 20],
  ["third.zip", 3 << 20],
  ["fourth.zip", 3 << 20],
  ["fifth.zip", 3 << 20],
  ["sixth.zip", 3 << 20],
  ["small.zip", 10 << 10],
  ["bottom.zip", 1 << 20],
]) {
  writeFileSync(join(www, "files", name), bytes(size, name.charCodeAt(0)));
}

// The fake native host, registered for the test profile only.
const wrapper = join(hostDir, "host.sh");
writeFileSync(wrapper, `#!/bin/sh\nexec python3 -I ${JSON.stringify(join(HERE, "fake-host.py"))} ${JSON.stringify(hostDir)} "$@"\n`);
chmodSync(wrapper, 0o755);
const setMode = (mode) => writeFileSync(join(hostDir, "mode"), mode);
setMode("ok");
mkdirSync(join(profile, "NativeMessagingHosts"), { recursive: true });
writeFileSync(
  join(profile, "NativeMessagingHosts", "com.pixidl.app.json"),
  JSON.stringify({ name: "com.pixidl.app", description: "fake pixidl host (e2e)", path: wrapper, type: "stdio", allowed_origins: [`chrome-extension://${EXT_ID}/`] }, null, 2),
);
mkdirSync(join(profile, "Default"), { recursive: true });
writeFileSync(join(profile, "Default", "Preferences"), JSON.stringify({ download: { default_directory: downloadDir, prompt_for_download: false, directory_upgrade: true } }));

function hostRequests() {
  const file = join(hostDir, "requests.jsonl");
  if (!existsSync(file)) return [];
  return readFileSync(file, "utf8").split("\n").filter(Boolean).map((line) => JSON.parse(line));
}
const addRequests = () => hostRequests().filter((r) => r.request.type === "add_download");

const port = await freePort();
const server = spawn("python3", ["-m", "http.server", String(port), "--bind", "127.0.0.1", "--directory", www], { stdio: "ignore" });
const ORIGIN = `http://127.0.0.1:${port}`;
const SITE = `${ORIGIN}/index.html`;
await waitFor(async () => {
  try {
    return (await fetch(SITE)).ok;
  } catch {
    return false;
  }
});

const context = await chromium.launchPersistentContext(profile, {
  headless: false,
  executablePath: process.env.CHROMIUM || "/opt/pw-browsers/chromium",
  args: [`--disable-extensions-except=${EXT_DIR}`, `--load-extension=${EXT_DIR}`, "--no-first-run", "--no-default-browser-check"],
  viewport: { width: 1200, height: 800 },
  acceptDownloads: true,
});

async function worker() {
  const find = () => context.serviceWorkers().find((w) => w.url().startsWith(`chrome-extension://${EXT_ID}/`));
  return find() ?? (await context.waitForEvent("serviceworker", { predicate: (w) => w.url().startsWith(`chrome-extension://${EXT_ID}/`), timeout: 15000 }));
}
async function swEval(fn, arg) {
  for (let attempt = 0; ; attempt++) {
    try {
      return await (await worker()).evaluate(fn, arg);
    } catch (err) {
      if (attempt >= 3) throw err;
      await sleep(500);
    }
  }
}

// Downloads must take the browser's normal path (Playwright otherwise
// intercepts them), so the extension's downloads API sees what a user's would.
async function useNormalDownloads(page) {
  const session = await context.newCDPSession(page);
  await session.send("Browser.setDownloadBehavior", { behavior: "default" }).catch((e) => console.log("setDownloadBehavior:", e.message));
  await session.detach().catch(() => {});
}

// ---- Looking into the extension's closed shadow root (through CDP) ------------------------

async function overlayState(page) {
  const cdp = await context.newCDPSession(page);
  try {
    const { root } = await cdp.send("DOM.getDocument", { depth: -1, pierce: true });
    const hosts = [];
    const walk = (n, fn) => {
      fn(n);
      for (const c of n.children || []) walk(c, fn);
      for (const s of n.shadowRoots || []) walk(s, fn);
      if (n.contentDocument) walk(n.contentDocument, fn);
    };
    walk(root, (n) => {
      if (n.nodeName === "PIXIDL-UI") hosts.push(n);
    });
    const attrs = (n) => {
      const out = {};
      for (let i = 0; i < (n.attributes || []).length; i += 2) out[n.attributes[i]] = n.attributes[i + 1];
      return out;
    };
    const text = (n) => {
      let s = "";
      walk(n, (m) => {
        if (m.nodeType === 3) s += m.nodeValue;
      });
      return s;
    };
    let button = null;
    let toast = null;
    for (const host of hosts) {
      walk(host, (n) => {
        const a = attrs(n);
        if (n.nodeName === "BUTTON" && /\bsel\b/.test(a.class || "")) button ??= { node: n, a };
        if (n.nodeName === "DIV" && /\btoast\b/.test(a.class || "")) toast ??= { node: n, a };
      });
    }
    const state = { hosts: hosts.length, visible: false, box: null, title: null, toast: null };
    if (button) {
      state.title = button.a.title ?? null;
      if (!("hidden" in button.a)) {
        try {
          const { model } = await cdp.send("DOM.getBoxModel", { nodeId: button.node.nodeId });
          const [x1, y1, x2, , , y3] = model.border;
          state.box = { x: x1, y: y1, width: x2 - x1, height: y3 - y1 };
          state.visible = true;
        } catch {
          state.visible = false;
        }
      }
    }
    if (toast && /\bshow\b/.test(toast.a.class || "")) state.toast = text(toast.node).trim();
    return state;
  } finally {
    await cdp.detach().catch(() => {});
  }
}

/** The visible end of the page selection: the last non-empty client rect. */
const selectionEnd = (page) =>
  page.evaluate(() => {
    const sel = getSelection();
    if (!sel.rangeCount) return null;
    const rects = [...sel.getRangeAt(sel.rangeCount - 1).getClientRects()].filter((r) => r.width > 0 && r.height > 0 && r.width < 600);
    const r = rects[rects.length - 1];
    return r ? { left: r.left, right: r.right, top: r.top, bottom: r.bottom } : null;
  });

async function dragSelect(page, selector, { from = 0.005, to = 0.995, line = 0.5 } = {}) {
  const box = await page.locator(selector).boundingBox();
  await page.mouse.move(box.x + box.width * from, box.y + box.height * line);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * ((from + to) / 2), box.y + box.height * line, { steps: 6 });
  await page.mouse.move(box.x + box.width * to, box.y + box.height * line, { steps: 6 });
  await page.mouse.up();
}

async function clearSelection(page) {
  await page.mouse.click(1150, 700); // empty area on the right
  await sleep(300);
}

const visibleSoon = (page) => waitFor(async () => (await overlayState(page)).visible && overlayState(page), { timeout: 3000 });
async function hiddenFor(page, ms = 700) {
  await sleep(ms);
  return !(await overlayState(page)).visible;
}

function nearEnd(state, end, { tolerance = 40 } = {}) {
  if (!state?.box || !end) return false;
  const centerY = state.box.y + state.box.height / 2;
  const endY = (end.top + end.bottom) / 2;
  const dx = state.box.x - end.right;
  return Math.abs(centerY - endY) <= 6 && dx >= 0 && dx <= tolerance;
}

// ---- Scenarios ---------------------------------------------------------------------------

const page = context.pages()[0] ?? (await context.newPage());
await worker();
await useNormalDownloads(page);
await page.goto(SITE);
await sleep(1200);

// Defaults on a fresh profile.
{
  const stored = await waitFor(() => swEval(async () => {
    const s = await chrome.storage.local.get(null);
    return s.settingsVersion ? s : null;
  }));
  check("fresh install: capture on by default, settings stamped v2", stored?.captureDownloads === true && stored?.settingsVersion === 2, JSON.stringify({ captureDownloads: stored?.captureDownloads, minSizeKb: stored?.minSizeKb, settingsVersion: stored?.settingsVersion }));
  const ping = await waitFor(() => swEval(async () => (await chrome.storage.local.get("lastPing")).lastPing?.connected === true));
  check("fresh install: background pinged the (fake) app", ping === true);
}

// 1. Mouse drag across a paragraph with two <a> links.
{
  await dragSelect(page, "#links");
  const state = await visibleSoon(page);
  const end = await selectionEnd(page);
  check("drag over links: button shown", state?.visible, state?.title);
  check("drag over links: button sits right after the selection", nearEnd(state, end), JSON.stringify({ box: state?.box, end }));
  check("drag over links: counts 2 links", /2/.test(state?.title || ""));
  await page.screenshot({ path: join(OUT_DIR, "selection-button-idle.png"), clip: { x: 0, y: 50, width: 700, height: 120 } });
  await page.mouse.move(state.box.x + state.box.width / 2, state.box.y + state.box.height / 2);
  await sleep(300);
  await page.screenshot({ path: join(OUT_DIR, "selection-button-hover.png"), clip: { x: 0, y: 50, width: 700, height: 120 } });
  await page.screenshot({ path: join(OUT_DIR, "selection-button-page.png") });
  await clearSelection(page);
  check("click elsewhere hides it", await hiddenFor(page));
}

// 2. Addresses typed as plain text (no <a>).
{
  // The paragraph wraps: drag from the start of line 1 to the end of line 2.
  const box = await page.locator("#plain").boundingBox();
  await page.mouse.move(box.x + 2, box.y + box.height * 0.25);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.75, { steps: 8 });
  await page.mouse.move(box.x + box.width - 2, box.y + box.height * 0.75, { steps: 8 });
  await page.mouse.up();
  const state = await visibleSoon(page);
  check("plain-text URLs: button shown", state?.visible, state?.title);
  check("plain-text URLs: https, magnet and https found (3 links)", /\b3\b/.test(state?.title || ""), state?.title);
  await clearSelection(page);
}

// 3. No address at all: no button.
{
  await dragSelect(page, "#nolink");
  check("selection without any URL: no button", await hiddenFor(page, 900));
  await clearSelection(page);
}

// 4. Selections that touch a single link. (Chromium does not select link text
// on double-click - pressing on a link starts a link drag - so users select
// into a link from the text before it.)
{
  const before = await page.locator("#dblwrap").boundingBox();
  const link = await page.locator("#spa").boundingBox();
  await page.mouse.move(before.x + 60, link.y + link.height / 2);
  await page.mouse.down();
  await page.mouse.move(link.x + link.width / 2, link.y + link.height / 2, { steps: 10 });
  await page.mouse.up();
  const state = await visibleSoon(page);
  const sel = await page.evaluate(() => String(getSelection()));
  check("drag ending inside a link: button shown", state?.visible, `selected "${sel}", ${state?.title}`);
  check("drag ending inside a link: one link", /\b1\b/.test(state?.title || ""));
  await clearSelection(page);
  // A word entirely inside the link (as Alt+drag or a script would select it).
  await page.evaluate(() => {
    const text = document.getElementById("spa").firstChild;
    const r = document.createRange();
    r.setStart(text, 0);
    r.setEnd(text, 9);
    getSelection().removeAllRanges();
    getSelection().addRange(r);
  });
  const inner = await visibleSoon(page);
  check("selection inside one link: button shown", inner?.visible && /\b1\b/.test(inner?.title || ""), inner?.title);
  await clearSelection(page);
  // Double-click on an ordinary word next to a link: no URL, no button.
  const nolink = await page.locator("#nolink").boundingBox();
  await page.mouse.dblclick(nolink.x + 30, nolink.y + nolink.height / 2);
  const word = await page.evaluate(() => String(getSelection()));
  check("double-click on a plain word: no button", word.length > 0 && (await hiddenFor(page, 700)), `selected "${word}"`);
  await clearSelection(page);
}

// 5. Triple-click a paragraph: the button goes after the text, not on the next line.
{
  await page.locator("#links").click({ clickCount: 3 });
  const state = await visibleSoon(page);
  const p = await page.locator("#links").boundingBox();
  const centerY = state?.box ? state.box.y + state.box.height / 2 : -1;
  check("triple-click: button shown", state?.visible);
  check("triple-click: button on the paragraph's line", centerY >= p.y && centerY <= p.y + p.height, JSON.stringify({ box: state?.box, paragraph: p }));
  await clearSelection(page);
}

// 6. Keyboard: double-click a word without a URL, then extend with Shift+End.
{
  await page.locator("#ftp").evaluate((el) => el.scrollIntoView({ block: "center" }));
  await page.evaluate(() => window.scrollTo(0, 0));
  const box = await page.locator("#plain").boundingBox();
  await page.mouse.dblclick(box.x + 20, box.y + box.height * 0.25); // "Mirrors:"
  const before = await hiddenFor(page, 600);
  await page.keyboard.press("Shift+End");
  await sleep(100);
  const sel = await page.evaluate(() => String(getSelection()));
  const state = await visibleSoon(page);
  check("keyboard: no button for the first word alone", before);
  check("keyboard: Shift+End over a URL shows the button", state?.visible, `selected "${sel.slice(0, 60)}"`);
  await clearSelection(page);
}

// 7. Escape hides the button and it stays hidden for that selection.
{
  await dragSelect(page, "#links");
  await visibleSoon(page);
  await page.keyboard.press("Escape");
  check("Escape hides the button", await hiddenFor(page, 600));
  await clearSelection(page);
}

// 8. Text fields: only when the selected text holds a URL.
{
  const ta = await page.locator("#ta").boundingBox();
  // Right to left, from after the text to the very start of the line.
  await page.mouse.move(ta.x + ta.width - 10, ta.y + 15);
  await page.mouse.down();
  await page.mouse.move(ta.x + 3, ta.y + 15, { steps: 10 });
  await page.mouse.up();
  const sel = await page.locator("#ta").evaluate((el) => el.value.slice(el.selectionStart, el.selectionEnd));
  const state = await visibleSoon(page);
  check("textarea with a URL selected: button shown", state?.visible, `selected "${sel}"`);
  await clearSelection(page);
  await page.locator("#inp").click({ clickCount: 3 });
  check("input without a URL selected: no button", await hiddenFor(page, 800));
  await clearSelection(page);
}

// 9. Scrolling moves the button with the selection.
{
  await dragSelect(page, "#links");
  const before = await visibleSoon(page);
  await page.mouse.move(600, 400);
  await page.mouse.wheel(0, 40);
  await sleep(400);
  const after = await overlayState(page);
  const end = await selectionEnd(page);
  const scrolled = await page.evaluate(() => window.scrollY);
  check("scroll: button still shown", after.visible && before?.visible, `scrollY=${scrolled}`);
  check("scroll: button follows the selection", nearEnd(after, end), JSON.stringify({ before: before?.box, after: after.box, end }));
  await page.evaluate(() => window.scrollTo(0, 0));
  await clearSelection(page);
}

// 10. RTL text.
{
  await page.locator("#rtl").click({ clickCount: 3 });
  const state = await visibleSoon(page);
  const p = await page.locator("#rtl").boundingBox();
  const inside = state?.box && state.box.x >= 0 && state.box.x + state.box.width <= 1200;
  check("RTL paragraph: button shown inside the viewport", state?.visible && inside, JSON.stringify({ box: state?.box, paragraph: p }));
  await clearSelection(page);
}

// 11. Page zoom 150 %.
{
  const tabId = await swEval(async (url) => (await chrome.tabs.query({ url }))[0]?.id, SITE);
  await swEval(async (id) => chrome.tabs.setZoom(id, 1.5), tabId);
  await sleep(500);
  await dragSelect(page, "#links");
  const state = await visibleSoon(page);
  const end = await selectionEnd(page);
  check("zoom 150%: button right after the selection", nearEnd(state, end), JSON.stringify({ box: state?.box, end }));
  await page.screenshot({ path: join(OUT_DIR, "selection-button-zoom150.png") });
  await clearSelection(page);
  await swEval(async (id) => chrome.tabs.setZoom(id, 1), tabId);
  await sleep(300);
}

// 12. A drag that never ends in pointerup (link drag-and-drop) does not block the button.
{
  const link = await page.locator("#dl2").boundingBox();
  await page.mouse.move(link.x + 10, link.y + link.height / 2);
  await page.mouse.down();
  await page.mouse.move(link.x + 80, link.y + 60, { steps: 8 });
  await page.mouse.move(link.x + 120, link.y + 90, { steps: 8 });
  await page.mouse.up();
  await sleep(300);
  // Now select with the keyboard only (no further pointer events).
  await page.evaluate(() => {
    const p = document.getElementById("links");
    getSelection().selectAllChildren(p);
  });
  const state = await visibleSoon(page);
  check("after a link drag, a script/keyboard selection still shows the button", state?.visible);
  await clearSelection(page);
}

// ---- Tabs that were open before the extension (re)started ---------------------------------

async function selectionWorksWithoutReload(label) {
  await page.bringToFront();
  await sleep(500);
  await dragSelect(page, "#links");
  const state = await visibleSoon(page);
  check(`${label}: open tab gets the button without a reload`, state?.visible, `pixidl-ui hosts=${state?.hosts}`);
  check(`${label}: exactly one copy of the UI (old one torn down)`, state?.hosts === 1, `hosts=${state?.hosts}`);
  // A copy left over from before is cut off from the extension: its strings
  // no longer resolve and its clicks go nowhere. The live one opens the picker.
  check(`${label}: the button is the live copy (localized title)`, /pixidl/.test(state?.title || ""), state?.title);
  if (state?.visible) {
    const picker = context.waitForEvent("page", { predicate: (p) => p.url().includes("picker.html"), timeout: 5000 }).catch(() => null);
    await page.mouse.click(state.box.x + state.box.width / 2, state.box.y + state.box.height / 2);
    const opened = await picker;
    check(`${label}: clicking the button opens the picker with the 2 links`, Boolean(opened));
    if (opened) {
      await opened.waitForLoadState().catch(() => {});
      await sleep(800);
      if (label === "after disable + enable") await opened.screenshot({ path: join(OUT_DIR, "picker.png") }).catch(() => {});
      await opened.close().catch(() => {});
    }
  }
  await page.bringToFront();
  await clearSelection(page);
}

// 13. The extension is reloaded (what an update does). This Chromium build
// then disables an extension loaded with --load-extension
// ("unsupportedDeveloperExtension"), so it is enabled again through
// chrome.management when that happens; either way the extension starts afresh
// while the test page stays open.
{
  const marker = await page.evaluate(() => (window.__e2eMarker = Math.random()));
  await swEval(() => {
    setTimeout(() => chrome.runtime.reload(), 50);
    return true;
  }).catch(() => {});
  await sleep(2500);
  const ext = await context.newPage();
  await ext.goto(`chrome://extensions/?id=${EXT_ID}`);
  const info = await ext.evaluate(async (id) => {
    const i = await chrome.developerPrivate.getExtensionInfo(id);
    return { state: i.state, reasons: Object.entries(i.disableReasons || {}).filter(([, v]) => v).map(([k]) => k) };
  }, EXT_ID);
  if (info.state !== "ENABLED") {
    console.log(`note: after reload the extension is ${info.state} (${info.reasons.join(", ")}); enabling it again`);
    await ext.evaluate((id) => chrome.management.setEnabled(id, true), EXT_ID);
    await sleep(2500);
  }
  await ext.close();
  const same = await page.evaluate(() => window.__e2eMarker);
  check("reload: test page was not reloaded", same === marker);
  await selectionWorksWithoutReload("after extension reload");
}

// 14. Disabled and enabled again. The switch on chrome://extensions turns it
// off, but cannot turn a command-line-loaded extension back on, so both steps
// go through chrome.management on that page (the API behind the switch).
{
  const ext = await context.newPage();
  await ext.goto(`chrome://extensions/?id=${EXT_ID}`);
  await ext.evaluate((id) => chrome.management.setEnabled(id, false), EXT_ID);
  await sleep(1500);
  // The cut-off copy must not show a dead button (it removes itself).
  await page.bringToFront();
  await dragSelect(page, "#links");
  await sleep(800);
  const offState = await overlayState(page);
  await clearSelection(page);
  await ext.evaluate((id) => chrome.management.setEnabled(id, true), EXT_ID);
  await sleep(2500);
  await ext.close();
  check("disabled: no button and no leftover UI in the open tab", !offState.visible && offState.hosts === 0, JSON.stringify(offState));
  await selectionWorksWithoutReload("after disable + enable");
}

// ---- Capturing downloads ----------------------------------------------------------------

async function downloadsFor(name) {
  return swEval(async (n) => (await chrome.downloads.search({})).filter((d) => d.url.endsWith(`/files/${n}`) || d.filename.endsWith(n)).map((d) => ({ id: d.id, state: d.state, error: d.error, filename: d.filename, exists: d.exists })), name);
}
async function setSettings(patch) {
  await swEval(async (p) => chrome.storage.local.set(p), patch);
  await sleep(200);
}
async function setPing(connected) {
  await swEval(
    async (c) =>
      chrome.storage.local.set({
        lastPing: c
          ? { connected: true, app_version: "9.9.9-fake", accent_color: "#7C3AED", integration_enabled: true, checked_at: Date.now() }
          : { connected: false, error_code: "app_unavailable", checked_at: Date.now() },
      }),
    connected,
  );
}
const fileOnDisk = (name) => readdirSync(downloadDir).some((f) => f === name || f.startsWith(name.replace(/\.zip$/, "")));
const completed = (name) => waitFor(async () => (await downloadsFor(name)).some((d) => d.state === "complete"), { timeout: 15000 });

await useNormalDownloads(page);
await page.goto(SITE);
await sleep(800);

// C1. pixidl accepts: browser download cancelled and erased, host got the URL.
{
  setMode("ok");
  await setPing(true);
  const before = addRequests().length;
  await page.locator("#dl").click();
  const sent = await waitFor(() => addRequests().length > before && addRequests().at(-1));
  const payload = sent?.request?.payload;
  check("capture: host received add_download with the URL", payload?.url === `${ORIGIN}/files/app.zip`, JSON.stringify(payload));
  check("capture: referrer and file name sent", payload?.referrer === SITE && payload?.filename === "app.zip");
  check("capture: client info sent", sent?.request?.client?.browser && sent?.request?.client?.version === "2.1.0", JSON.stringify(sent?.request?.client));
  const toast = await waitFor(async () => (await overlayState(page)).toast, { timeout: 4000 });
  check("capture: 'Sent to pixidl' notice on the page", /pixidl/.test(toast || "") && /app\.zip/.test(toast || ""), toast);
  await page.screenshot({ path: join(OUT_DIR, "capture-toast.png") });
  await sleep(1500);
  const items = await downloadsFor("app.zip");
  check("capture: browser download cancelled and erased", items.length === 0 || items.every((d) => d.state === "interrupted"), JSON.stringify(items));
  check("capture: no file written by the browser", !fileOnDisk("app.zip"), readdirSync(downloadDir).join(","));
}

// C2. Plain link to a .zip (no download attribute) is captured too.
{
  const before = addRequests().length;
  await page.locator("#dl2").click();
  const sent = await waitFor(() => addRequests().length > before && addRequests().at(-1));
  check("capture: plain .zip link captured", sent?.request?.payload?.url === `${ORIGIN}/files/second.zip`);
  await sleep(1000);
  check("capture: plain .zip link not saved by the browser", !fileOnDisk("second.zip"));
}

async function clickDownload(selector, href) {
  await page.evaluate(([sel, h]) => {
    const a = document.querySelector(sel);
    if (h) a.setAttribute("href", h);
  }, [selector, href]);
  await page.locator(selector).click();
}

// C3. The host answers add_download with an error: the browser finishes it.
{
  setMode("add_fail");
  await setPing(true);
  const before = addRequests().length;
  await clickDownload("#dl", "files/third.zip");
  const asked = await waitFor(() => addRequests().length > before);
  const done = await completed("third.zip");
  check("fallback (add_download fails): pixidl was asked", asked);
  check("fallback (add_download fails): browser download completes", done && fileOnDisk("third.zip"), JSON.stringify(await downloadsFor("third.zip")));
}

// C4. The host answers app_unavailable: the browser finishes it.
{
  setMode("unavailable");
  await setPing(true);
  const before = addRequests().length;
  await clickDownload("#dl", "files/fourth.zip");
  const asked = await waitFor(() => addRequests().length > before);
  const done = await completed("fourth.zip");
  check("fallback (app_unavailable): pixidl was asked", asked);
  check("fallback (app_unavailable): browser download completes", done && fileOnDisk("fourth.zip"));
  // Right after that failure the next download is not held at all.
  const before2 = hostRequests().filter((r) => r.request.type === "add_download").length;
  await clickDownload("#dl", "files/fifth.zip");
  const done2 = await completed("fifth.zip");
  check("fallback: next download right after is left to the browser without asking", done2 && addRequests().length === before2);
}

// C5. A broken host (exits without answering).
{
  setMode("crash");
  await setPing(true);
  const before = hostRequests().length;
  await clickDownload("#dl", "files/sixth.zip");
  const done = await completed("sixth.zip");
  check("fallback (host crashes): browser download completes", done && hostRequests().length > before);
}

// C6-C10. Downloads that are never captured.
setMode("ok");
async function notCaptured(label, action, name) {
  await setPing(true);
  const before = addRequests().length;
  await action();
  const done = await completed(name);
  check(`not captured: ${label}`, done && addRequests().length === before, `asked=${addRequests().length - before}`);
}
await notCaptured("file below the 100 KB minimum", () => page.locator("#small").click(), "small.zip");
await notCaptured("blob: download", () => page.locator("#blob").click(), "generated.bin");
{
  await setPing(true);
  await swEval(async () => {
    for (const d of await chrome.downloads.search({})) if (d.url.endsWith("/files/bottom.zip")) await chrome.downloads.erase({ id: d.id });
  });
  await page.evaluate(() => document.querySelector("#dl").setAttribute("href", "files/bottom.zip"));
  const before = addRequests().length;
  await page.locator("#dl").click({ modifiers: ["Alt"] });
  const done = await completed("bottom.zip");
  check("not captured: Alt+click on a link", done && addRequests().length === before, `asked=${addRequests().length - before}`);
}
{
  await setSettings({ captureExcludedSites: ["127.0.0.1"] });
  await swEval(async () => {
    for (const d of await chrome.downloads.search({})) await chrome.downloads.erase({ id: d.id });
  });
  for (const f of readdirSync(downloadDir)) rmSync(join(downloadDir, f), { force: true });
  await notCaptured("site excluded", () => clickDownload("#dl", "files/app.zip"), "app.zip");
  await setSettings({ captureExcludedSites: [] });
}
{
  await setSettings({ captureDownloads: false });
  await swEval(async () => {
    for (const d of await chrome.downloads.search({})) await chrome.downloads.erase({ id: d.id });
  });
  for (const f of readdirSync(downloadDir)) rmSync(join(downloadDir, f), { force: true });
  await notCaptured("capture turned off", () => clickDownload("#dl", "files/second.zip"), "second.zip");
  await setSettings({ captureDownloads: true });
}

// ---- Popup and options (screenshots + per-site switch) -----------------------------------

{
  await setPing(true);
  await page.bringToFront();
  // action.openPopup() needs a focused window, which Xvfb without a window
  // manager does not give; popup.html?tab=<id> renders the same popup for the
  // test page in a tab of its own (380 px wide like the real popup).
  const tabId = await swEval(async (url) => (await chrome.tabs.query({ url }))[0]?.id, SITE);
  const popup = await context.newPage();
  await popup.setViewportSize({ width: 370, height: 560 });
  await popup.goto(`chrome-extension://${EXT_ID}/popup.html?tab=${tabId}`);
  await sleep(1500);
  await popup.screenshot({ path: join(OUT_DIR, "popup-connected.png") });
  const siteLabel = await popup.locator("#site-label").textContent();
  check("popup: capture switch shown and on", (await popup.locator("#capture").isVisible()) && (await popup.locator("#capture").isChecked()));
  check("popup: per-site switch for this site", /127\.0\.0\.1/.test(siteLabel || ""), siteLabel);
  check("popup: connected status", /Connected/.test((await popup.locator("#conn-text").textContent()) || ""));
  await popup.locator("#capture-site").click();
  await sleep(400);
  const sites = await swEval(async () => (await chrome.storage.local.get("captureExcludedSites")).captureExcludedSites);
  check("popup: turning the site off stores the exclusion", Array.isArray(sites) && sites.includes("127.0.0.1"), JSON.stringify(sites));
  await popup.screenshot({ path: join(OUT_DIR, "popup-site-off.png") });
  await popup.locator("#capture-site").click();
  await sleep(400);
  const after = await swEval(async () => (await chrome.storage.local.get("captureExcludedSites")).captureExcludedSites);
  check("popup: turning it back on removes the exclusion", Array.isArray(after) && after.length === 0, JSON.stringify(after));
  await popup.locator("#capture").click();
  await sleep(300);
  const off = await swEval(async () => (await chrome.storage.local.get("captureDownloads")).captureDownloads);
  check("popup: main capture switch saves", off === false);
  await popup.locator("#capture").click();
  await sleep(300);
  await popup.close();
  // Not connected: the one-line hint.
  setMode("unavailable");
  const tab = await context.newPage();
  await tab.setViewportSize({ width: 380, height: 640 });
  await tab.goto(`chrome-extension://${EXT_ID}/popup.html?tab=${tabId}`);
  await sleep(1500);
  const hint = await tab.locator("#conn-hint").textContent();
  check("popup: not connected shows the install/open hint", (await tab.locator("#conn-hint").isVisible()) && /pixidl/.test(hint || ""), hint);
  await tab.screenshot({ path: join(OUT_DIR, "popup-not-connected.png") });
  setMode("ok");
  await tab.setViewportSize({ width: 640, height: 900 });
  await tab.goto(`chrome-extension://${EXT_ID}/options.html`);
  await sleep(1000);
  await tab.screenshot({ path: join(OUT_DIR, "options.png"), fullPage: true });
  await tab.close();
}

// ---- Done ---------------------------------------------------------------------------------

await context.close();
server.kill();
const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed. Screenshots: ${OUT_DIR}`);
if (failed.length) {
  for (const f of failed) console.log(`  FAILED: ${f.name} ${f.detail}`);
  process.exitCode = 1;
}
if (!process.env.KEEP_E2E_TMP) rmSync(tmp, { recursive: true, force: true });

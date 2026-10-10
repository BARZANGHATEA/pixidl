// Background script (Chromium service worker / Firefox event page).
// Owns the native-messaging calls made on behalf of content scripts, the
// context menus, the toolbar badge, the picker window, the periodic ping and
// the capture of browser downloads. Listeners are registered synchronously at
// top level so the browser can wake the worker for them.

import { api, errorText, pingAndCache, cachedPing, send, t } from "./native.js";
import { loadSettings, migrateStoredSettings } from "./settings.js";
import {
  BYPASS_TTL_MS,
  MAX_PAGE_LINKS,
  badgeText,
  basename,
  buildItem,
  captureDecision,
  extractUrlsFromText,
  isHttpUrl,
  normalizeEntries,
  pingSummary,
  safeAccent,
  supportedUrl,
  urlFileName,
} from "./lib.js";

const PING_ALARM = "pixidl-ping";
const PING_MINUTES = 10;
const PICKER_PREFIX = "picker:";
const PICKER_TTL_MS = 5 * 60 * 1000;

const MENU = Object.freeze({
  link: "pixidl-download-link",
  media: "pixidl-download-media",
  selection: "pixidl-download-selection",
  page: "pixidl-send-page",
  allLinks: "pixidl-all-links",
});

const isFirefox = typeof api.runtime.getBrowserInfo === "function";
/** Session storage (cleared when the browser closes); local storage as a fallback. */
const sessionArea = () => api.storage.session ?? api.storage.local;

// ---- Lifecycle -----------------------------------------------------------------

async function setupMenus() {
  await api.contextMenus.removeAll();
  const items = [
    { id: MENU.link, title: t("menuDownloadLink"), contexts: ["link"] },
    { id: MENU.media, title: t("menuDownloadMedia"), contexts: ["video", "audio", "image"] },
    { id: MENU.selection, title: t("menuDownloadSelection"), contexts: ["selection"] },
    { id: MENU.page, title: t("menuSendPage"), contexts: ["page"] },
    { id: MENU.allLinks, title: t("menuAllLinks"), contexts: ["page"] },
  ];
  for (const item of items) api.contextMenus.create(item, () => void api.runtime.lastError);
}

async function ensureAlarm() {
  try {
    const existing = await api.alarms.get(PING_ALARM);
    if (!existing) await api.alarms.create(PING_ALARM, { periodInMinutes: PING_MINUTES });
  } catch {
    // Alarms are only used to keep the app's "connected" view fresh.
  }
}

/**
 * Chromium runs manifest content scripts only in pages loaded after the
 * extension started, so tabs that were open when it was installed, updated,
 * reloaded or enabled again would get no selection button until reloaded.
 * The content script ignores a second copy of itself.
 */
async function injectIntoOpenTabs() {
  if (isFirefox) return; // Firefox injects them into open tabs itself.
  let tabs = [];
  try {
    tabs = await api.tabs.query({ url: ["http://*/*", "https://*/*"] });
  } catch {
    return; // Not fatal: the script runs after the next reload of each tab.
  }
  await Promise.all(
    tabs
      .filter((tab) => !tab.discarded && tab.id != null && tab.id >= 0)
      .map((tab) => api.scripting.executeScript({ target: { tabId: tab.id }, files: ["content.js"] }).catch(() => {})),
  );
}

/**
 * Runs once each time the extension starts (install, update, reload, enable,
 * browser start). storage.session is emptied whenever the extension stops, so
 * a service worker that merely wakes up again does not repeat the work.
 */
async function onExtensionStart() {
  await migrateStoredSettings();
  try {
    const area = api.storage.session;
    if (area) {
      const { started } = await area.get("started");
      if (started) return;
      await area.set({ started: Date.now() });
    }
  } catch {
    // Without session storage the work below simply runs again.
  }
  // Enabling the extension again fires neither onInstalled nor onStartup.
  await setupMenus();
  await ensureAlarm();
  await injectIntoOpenTabs();
  pingAndCache();
}

api.runtime.onInstalled.addListener(async () => {
  await migrateStoredSettings();
  await setupMenus();
  await ensureAlarm();
});

api.runtime.onStartup.addListener(async () => {
  await setupMenus();
  await ensureAlarm();
});

api.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === PING_ALARM) pingAndCache();
});

// ---- Feedback ------------------------------------------------------------------

/** System notification; failures to notify are not fatal. */
function notify(ok, message) {
  try {
    Promise.resolve(
      api.notifications.create({
        type: "basic",
        iconUrl: api.runtime.getURL("icons/icon-128.png"),
        title: ok ? t("notifySentTitle") : t("notifyFailedTitle"),
        message: String(message || ""),
      }),
    ).catch(() => {});
  } catch {
    // Notifications may be disabled at the OS level.
  }
}

/** Shows the in-page toast in the tab when its content script is there, else a notification. */
async function report(tabId, ok, text) {
  if (tabId != null && tabId >= 0) {
    try {
      const shown = await api.tabs.sendMessage(tabId, { type: "pixidl:toast", ok, text }, { frameId: 0 });
      if (shown === true) return;
    } catch {
      // No content script in this tab (e.g. a browser page); fall through.
    }
  }
  notify(ok, text);
}

/** add_download for one URL; returns `{success, text}` with a localized outcome. */
async function addOne(rawUrl, { referrer, engine } = {}) {
  const url = supportedUrl(rawUrl);
  if (!url) return { success: false, text: t("errInvalidUrl") };
  const resp = await send("add_download", buildItem(url, { referrer, engine }));
  if (!resp.success) return { success: false, code: resp.error.code, text: errorText(resp.error) };
  return { success: true, text: t("sentOne", [resp.filename || url]) };
}

/** open_in_app: the app comes to the front with its Add dialog pre-filled. */
async function openInApp(rawUrl, referrer) {
  const url = supportedUrl(rawUrl);
  if (!url) return { success: false, text: t("errInvalidUrl") };
  const payload = buildItem(url, { referrer });
  delete payload.filename;
  const resp = await send("open_in_app", payload);
  if (!resp.success) return { success: false, code: resp.error.code, text: errorText(resp.error) };
  return { success: true, text: t("openedInApp") };
}

// ---- Picker window -------------------------------------------------------------

async function dropStalePickerData() {
  try {
    const all = await sessionArea().get(null);
    const stale = Object.entries(all)
      .filter(([key, value]) => key.startsWith(PICKER_PREFIX) && !(Date.now() - (value?.created ?? 0) < PICKER_TTL_MS))
      .map(([key]) => key);
    if (stale.length) await sessionArea().remove(stale);
  } catch {
    // Best effort.
  }
}

/** Opens picker.html in a small popup window with the given links. */
async function openPicker(links, { pageUrl = "", title = "" } = {}) {
  await dropStalePickerData();
  const id = crypto.randomUUID();
  await sessionArea().set({
    [PICKER_PREFIX + id]: { links, pageUrl: safeReferrerOrEmpty(pageUrl), title: String(title || "").slice(0, 300), created: Date.now() },
  });
  const options = { url: api.runtime.getURL(`picker.html?s=${id}`), type: "popup", width: 560, height: 640 };
  try {
    const win = await api.windows.getLastFocused();
    if (Number.isFinite(win?.left) && Number.isFinite(win?.width)) {
      options.left = Math.max(0, Math.round(win.left + (win.width - options.width) / 2));
      options.top = Math.max(0, Math.round(win.top + Math.max(0, (win.height - options.height) / 3)));
    }
  } catch {
    // Let the browser place the window.
  }
  await api.windows.create(options);
}

function safeReferrerOrEmpty(url) {
  return isHttpUrl(url) ? supportedUrl(url) : "";
}

// Runs in the page (serialized by scripting.executeScript): must be self-contained.
function pageLinksInPage(max) {
  const out = [];
  const push = (url, label) => {
    if (typeof url === "string" && url && out.length < max) out.push({ url, label: String(label || "").trim().slice(0, 200) });
  };
  for (const a of document.querySelectorAll("a[href], area[href]")) push(a.href, a.textContent || a.title || a.getAttribute("aria-label"));
  for (const m of document.querySelectorAll("video[src], audio[src], source[src]")) push(m.src, m.title);
  return { links: out, title: document.title };
}

// Runs in the page: links inside the current selection plus URLs typed in it.
function selectionLinksInPage() {
  const links = [];
  const selection = document.getSelection();
  for (let i = 0; selection && i < selection.rangeCount; i++) {
    const range = selection.getRangeAt(i);
    const node = range.commonAncestorContainer;
    const root = node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
    if (!root) continue;
    const enclosing = root.closest("a[href]");
    if (enclosing) links.push({ url: enclosing.href, label: enclosing.textContent });
    for (const a of root.querySelectorAll("a[href], area[href]")) {
      if (range.intersectsNode(a)) links.push({ url: a.href, label: a.textContent });
      if (links.length >= 5000) break;
    }
  }
  return { links, text: selection ? String(selection).slice(0, 200000) : "", title: document.title };
}

async function runInTab(tabId, frameId, func, args = []) {
  const [result] = await api.scripting.executeScript({ target: { tabId, frameIds: [frameId || 0] }, func, args });
  return result?.result ?? null;
}

async function openAllLinks(tabId) {
  const tab = await api.tabs.get(tabId);
  let data;
  try {
    data = await runInTab(tabId, 0, pageLinksInPage, [MAX_PAGE_LINKS]);
  } catch {
    return report(tabId, false, t("errCannotAccessPage"));
  }
  const links = normalizeEntries(data?.links, { exclude: [tab.url], limit: MAX_PAGE_LINKS });
  if (links.length === 0) return report(tabId, false, t("noLinksOnPage"));
  return openPicker(links, { pageUrl: tab.url, title: data?.title || tab.title });
}

async function openDetected(tabId) {
  const tab = await api.tabs.get(tabId);
  let detected = null;
  try {
    detected = await api.tabs.sendMessage(tabId, { type: "pixidl:getDetected" }, { frameId: 0 });
  } catch {
    detected = null;
  }
  const links = normalizeEntries(detected?.links);
  if (links.length === 0) return report(tabId, false, t("noLinksOnPage"));
  return openPicker(links, { pageUrl: tab.url, title: tab.title });
}

/** One link is sent right away; several open the picker. */
async function sendLinks(tabId, links, { pageUrl, title }) {
  if (links.length === 0) return report(tabId, false, t("noLinksInSelection"));
  if (links.length === 1) {
    const result = await addOne(links[0].url, { referrer: pageUrl });
    return report(tabId, result.success, result.text);
  }
  return openPicker(links, { pageUrl, title });
}

async function selectionFromMenu(info, tab) {
  let data;
  try {
    data = await runInTab(tab.id, info.frameId, selectionLinksInPage);
  } catch {
    // Restricted frame: fall back to the text the browser gives us.
    data = { links: [], text: info.selectionText || "" };
  }
  const pageUrl = info.frameUrl || info.pageUrl || tab.url;
  const entries = [...(data?.links ?? []), ...extractUrlsFromText(data?.text || "")];
  const links = normalizeEntries(entries, { exclude: [pageUrl] });
  return sendLinks(tab.id, links, { pageUrl, title: data?.title || tab.title });
}

// ---- Context menus -------------------------------------------------------------

api.contextMenus.onClicked.addListener(async (info, tab) => {
  const referrer = info.frameUrl || info.pageUrl;
  const tabId = tab?.id;
  switch (info.menuItemId) {
    case MENU.link: {
      const r = await addOne(info.linkUrl, { referrer });
      return report(tabId, r.success, r.text);
    }
    case MENU.media: {
      const r = await addOne(info.srcUrl, { referrer });
      return report(tabId, r.success, r.text);
    }
    case MENU.page: {
      const r = await openInApp(info.pageUrl || tab?.url);
      return report(tabId, r.success, r.text);
    }
    case MENU.allLinks:
      return tabId != null ? openAllLinks(tabId) : undefined;
    case MENU.selection:
      return tabId != null ? selectionFromMenu(info, tab) : undefined;
    default:
      return undefined;
  }
});

// ---- Toolbar badge ---------------------------------------------------------------

async function getCounts() {
  try {
    return (await sessionArea().get("tabCounts")).tabCounts ?? {};
  } catch {
    return {};
  }
}

async function setCount(tabId, count) {
  const counts = await getCounts();
  if (count > 0) counts[tabId] = count;
  else delete counts[tabId];
  try {
    await sessionArea().set({ tabCounts: counts });
  } catch {
    // Best effort.
  }
}

async function paintBadge(tabId, count, settings) {
  const text = settings.showBadge ? badgeText(count) : "";
  try {
    await api.action.setBadgeText({ tabId, text });
    if (text) {
      const accent = safeAccent((await cachedPing())?.accent_color);
      await api.action.setBadgeBackgroundColor({ tabId, color: accent });
      if (api.action.setBadgeTextColor) await api.action.setBadgeTextColor({ tabId, color: "#FFFFFF" });
    }
  } catch {
    // The tab may be gone.
  }
}

async function repaintAllBadges() {
  const settings = await loadSettings();
  const counts = await getCounts();
  for (const [tabId, count] of Object.entries(counts)) await paintBadge(Number(tabId), count, settings);
}

api.tabs.onUpdated.addListener((tabId, changeInfo) => {
  // A new page (or an in-page navigation) starts with a clean badge; its
  // content script reports again after scanning.
  if (changeInfo.url) {
    setCount(tabId, 0);
    paintBadge(tabId, 0, { showBadge: false });
  }
});

api.tabs.onRemoved.addListener((tabId) => setCount(tabId, 0));

api.storage.onChanged.addListener((changes, area) => {
  if (area === "local" && (changes.showBadge || changes.lastPing)) repaintAllBadges();
});

// ---- Messages from content scripts and extension pages ---------------------------

// Extension pages (popup, picker, options) have our own origin; content scripts report the page's URL.
const fromExtensionPage = (sender) => typeof sender.url === "string" && sender.url.startsWith(api.runtime.getURL(""));

async function handleMessage(msg, sender) {
  const tab = sender.tab;
  switch (msg?.type) {
    // From content scripts (always tied to a tab).
    case "pixidl:add":
      if (!tab || fromExtensionPage(sender)) return null;
      return addOne(msg.url, { referrer: sender.url || tab.url, engine: msg.engine === "video" ? "video" : undefined });
    case "pixidl:openInApp":
      if (!tab) return null;
      return openInApp(msg.url, sender.url || tab.url);
    case "pixidl:openPicker": {
      if (!tab) return null;
      const links = normalizeEntries(msg.links, { limit: MAX_PAGE_LINKS });
      if (links.length === 0) return { success: false, text: t("noLinksInSelection") };
      await openPicker(links, { pageUrl: sender.url || tab.url, title: tab.title });
      return { success: true };
    }
    case "pixidl:detected": {
      if (!tab || sender.frameId !== 0) return null;
      const count = Math.max(0, Math.floor(Number(msg.count)) || 0);
      await setCount(tab.id, count);
      await paintBadge(tab.id, count, await loadSettings());
      return { success: true };
    }
    case "pixidl:bypassCapture":
      // Alt+click on a link: the browser keeps that download.
      if (!tab) return null;
      await addBypass(msg.url);
      return { success: true };
    // From the popup.
    case "pixidl:reviewDetected":
      if (!fromExtensionPage(sender) || !Number.isInteger(msg.tabId)) return null;
      await openDetected(msg.tabId);
      return { success: true };
    case "pixidl:allLinks":
      if (!fromExtensionPage(sender) || !Number.isInteger(msg.tabId)) return null;
      await openAllLinks(msg.tabId);
      return { success: true };
    default:
      return null;
  }
}

api.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  if (sender.id !== api.runtime.id) return false;
  handleMessage(msg, sender).then(
    (result) => sendResponse(result),
    (err) => sendResponse({ success: false, text: String(err?.message || t("errUnknown")) }),
  );
  return true; // async response
});

// ---- Capturing browser downloads ---------------------------------------------------
//
// On by default: a download started in the browser goes to pixidl instead.
// pixidl must accept it first; whenever it cannot (app missing, integration
// off, any error or no answer in time) the browser keeps the download.
//
// Chromium: onDeterminingFilename holds the download before it can finish
//   (its data waits in a temporary file), so pixidl is asked while nothing has
//   been saved yet. Accepted: the browser copy is cancelled and erased.
//   Otherwise the hold is released and the browser carries on as usual.
// Firefox: there is no way to hold a download, so onCreated cancels it at once
//   and, if pixidl does not take it, starts it again in the browser (that
//   repeated download is marked as ours and never captured again).

/** Longest wait for pixidl (the native host may first have to start the app, up to 20 s). */
const CAPTURE_TIMEOUT_MS = 25 * 1000;
const BYPASS_KEY = "captureBypass";
const OWN_KEY = "ownDownloads";
const OWN_TTL_MS = 60 * 1000;

let settingsCache = null;
const currentSettings = () => (settingsCache ??= loadSettings());

api.storage.onChanged.addListener((changes, area) => {
  if (area === "local" && Object.keys(changes).some((key) => key !== "lastPing")) settingsCache = null;
});

/** A short-lived list in session storage: `[{url, until}]` with expired entries dropped. */
async function readList(key) {
  try {
    const list = (await sessionArea().get(key))[key];
    return Array.isArray(list) ? list.filter((e) => e && Number(e.until) > Date.now()) : [];
  } catch {
    return [];
  }
}

async function pushToList(key, url, ttl) {
  const clean = supportedUrl(url);
  if (!clean) return;
  const list = (await readList(key)).slice(-50);
  list.push({ url: clean, until: Date.now() + ttl });
  try {
    await sessionArea().set({ [key]: list });
  } catch {
    // Best effort.
  }
}

const addBypass = (url) => pushToList(BYPASS_KEY, url, BYPASS_TTL_MS);

let lastBackgroundPing = 0;
/** Refreshes the cached ping now and then while pixidl looks unreachable. */
function refreshPingSoon() {
  if (Date.now() - lastBackgroundPing < 30 * 1000) return;
  lastBackgroundPing = Date.now();
  pingAndCache();
}

/** Remembers that pixidl could not be reached, so the next downloads are not held. */
async function rememberUnreachable(resp) {
  try {
    const { lastPing } = await api.storage.local.get("lastPing");
    await api.storage.local.set({ lastPing: { ...pingSummary(resp), accent_color: lastPing?.accent_color } });
  } catch {
    // Best effort.
  }
}

async function decide(item) {
  const [settings, ping, bypass, own] = await Promise.all([currentSettings(), cachedPing(), readList(BYPASS_KEY), readList(OWN_KEY)]);
  const decision = captureDecision(item, { settings, ping, bypass, ownUrls: own.map((e) => e.url), now: Date.now() });
  if (decision.reason === "app_unreachable") refreshPingSoon();
  return { ...decision, settings };
}

function captureItem(item, url) {
  // Chromium's item.filename is the suggested name here (a full path in Firefox).
  const name = basename(item.filename) || urlFileName(url);
  return buildItem(url, { filename: name, referrer: item.referrer });
}

async function activeTabId() {
  try {
    const [tab] = await api.tabs.query({ active: true, lastFocusedWindow: true });
    return tab?.id;
  } catch {
    return undefined;
  }
}

async function captureNotice(settings, ok, text) {
  if (!settings.captureNotice) return;
  await report(await activeTabId(), ok, text);
}

async function cancelAndErase(id) {
  try {
    await api.downloads.cancel(id);
  } catch {
    // Already finished or gone.
  }
  try {
    await api.downloads.erase({ id });
  } catch {
    // Not in the list any more.
  }
}

async function downloadState(id) {
  try {
    const [found] = await api.downloads.search({ id });
    return found?.state ?? null;
  } catch {
    return null;
  }
}

/** Sends one download to pixidl; resolves `{resp, late}` or `{timedOut: true}` after CAPTURE_TIMEOUT_MS. */
function offerToApp(payload, onLateSuccess) {
  return new Promise((resolve) => {
    let settled = false;
    const timer = setTimeout(() => {
      settled = true;
      resolve({ timedOut: true });
    }, CAPTURE_TIMEOUT_MS);
    send("add_download", payload).then((resp) => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        resolve({ resp });
      } else if (resp.success) {
        onLateSuccess(resp);
      }
    });
  });
}

async function onCaptureFailed(resp, settings) {
  if (resp.error?.code === "app_unavailable" || resp.error?.code === "unauthorized") await rememberUnreachable(resp);
  await captureNotice(settings, false, t("captureFallback", [errorText(resp.error)]));
}

/** Chromium: decide while the browser waits for a file name. */
async function captureHeld(item, suggest) {
  let released = false;
  const release = () => {
    if (released) return;
    released = true;
    try {
      suggest();
    } catch {
      // The download is already gone.
    }
  };
  try {
    const decision = await decide(item);
    if (!decision.capture) return release();
    const payload = captureItem(item, decision.url);
    const outcome = await offerToApp(payload, async (resp) => {
      // pixidl answered after the browser had been let go: it has the file
      // now, so stop the browser's copy if it is still running.
      if ((await downloadState(item.id)) === "in_progress") await cancelAndErase(item.id);
      await captureNotice(decision.settings, true, t("capturedToast", [resp.filename || payload.filename || decision.url]));
    });
    if (outcome.timedOut) return release();
    const { resp } = outcome;
    if (!resp.success) {
      release();
      return onCaptureFailed(resp, decision.settings);
    }
    released = true; // never let this one finish in the browser
    await cancelAndErase(item.id);
    await captureNotice(decision.settings, true, t("capturedToast", [resp.filename || payload.filename || decision.url]));
  } catch {
    release();
  }
}

/** Firefox: cancel first, then ask pixidl; start it again in the browser if pixidl does not take it. */
async function captureCreated(item) {
  const decision = await decide(item);
  if (!decision.capture) return;
  try {
    await api.downloads.cancel(item.id);
  } catch {
    return; // finished already: the browser keeps it
  }
  if ((await downloadState(item.id)) === "complete") return;
  const payload = captureItem(item, decision.url);
  const outcome = await offerToApp(payload, () => {});
  const resp = outcome.timedOut ? { success: false, error: { code: "internal", message: "pixidl did not answer in time" } } : outcome.resp;
  if (resp.success) {
    try {
      await api.downloads.erase({ id: item.id });
    } catch {
      // Already removed.
    }
    await captureNotice(decision.settings, true, t("capturedToast", [resp.filename || payload.filename || decision.url]));
    return;
  }
  await restartInBrowser(item);
  await onCaptureFailed(resp, decision.settings);
}

async function restartInBrowser(item) {
  const url = typeof item.url === "string" ? item.url : "";
  if (!isHttpUrl(url)) return;
  await pushToList(OWN_KEY, url, OWN_TTL_MS);
  const options = { url, conflictAction: "uniquify", saveAs: false };
  const name = basename(item.filename);
  if (name) options.filename = name;
  try {
    await api.downloads.download(options);
  } catch {
    try {
      delete options.filename;
      await api.downloads.download(options);
    } catch {
      return;
    }
  }
  try {
    await api.downloads.erase({ id: item.id });
  } catch {
    // Keep the cancelled entry; the new one carries on.
  }
}

if (api.downloads.onDeterminingFilename) {
  api.downloads.onDeterminingFilename.addListener((item, suggest) => {
    captureHeld(item, suggest);
    return true; // suggest() is called asynchronously (or never, once pixidl took it)
  });
} else {
  api.downloads.onCreated.addListener((item) => {
    captureCreated(item).catch(() => {});
  });
}

ensureAlarm();
onExtensionStart();

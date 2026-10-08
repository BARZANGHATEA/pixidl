// Background script (Chromium service worker / Firefox event page).
// Owns the native-messaging calls made on behalf of content scripts, the
// context menus, the toolbar badge, the picker window, the periodic ping and
// the optional capture of browser downloads. Listeners are registered
// synchronously at top level so the browser can wake the worker for them.

import { api, errorText, pingAndCache, cachedPing, send, t } from "./native.js";
import { loadSettings } from "./settings.js";
import {
  MAX_PAGE_LINKS,
  badgeText,
  basename,
  buildItem,
  extractUrlsFromText,
  isHttpUrl,
  meetsMinSize,
  normalizeEntries,
  safeAccent,
  supportedUrl,
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

/** Chromium does not run manifest content scripts in tabs that were already open. */
async function injectIntoOpenTabs() {
  if (isFirefox) return; // Firefox injects them into open tabs itself.
  try {
    const tabs = await api.tabs.query({ url: ["http://*/*", "https://*/*"] });
    for (const tab of tabs) {
      if (tab.discarded) continue;
      api.scripting.executeScript({ target: { tabId: tab.id }, files: ["content.js"] }).catch(() => {});
    }
  } catch {
    // Not fatal: the script runs after the next reload of each tab.
  }
}

api.runtime.onInstalled.addListener(async (details) => {
  await setupMenus();
  await ensureAlarm();
  if (details.reason === "install" || details.reason === "update") injectIntoOpenTabs();
  pingAndCache();
});

api.runtime.onStartup.addListener(async () => {
  await setupMenus();
  await ensureAlarm();
  pingAndCache();
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

// ---- Optional capture of browser downloads ---------------------------------------
// pixidl is asked first; the browser download is cancelled only after pixidl
// accepted it, so any failure leaves the browser to finish it as usual.

api.downloads.onCreated.addListener(async (item) => {
  const settings = await loadSettings();
  if (!settings.captureDownloads) return;
  if (item.incognito || item.state !== "in_progress") return;
  if (item.byExtensionId && item.byExtensionId === api.runtime.id) return;
  // blob:, data:, file: and similar never leave the browser.
  if (!isHttpUrl(item.url)) return;
  const url = supportedUrl(isHttpUrl(item.finalUrl) ? item.finalUrl : item.url);
  const size = item.totalBytes > 0 ? item.totalBytes : item.fileSize;
  if (!meetsMinSize(size, settings.minSizeMb)) return;

  const resp = await send("add_download", buildItem(url, { filename: basename(item.filename), referrer: item.referrer }));
  if (!resp.success) {
    notify(false, t("captureFallback", [errorText(resp.error)]));
    return;
  }
  try {
    await api.downloads.cancel(item.id);
    await api.downloads.erase({ id: item.id });
  } catch {
    // The browser download already finished or was removed; nothing to undo.
  }
  notify(true, resp.filename || basename(item.filename) || url);
});

ensureAlarm();

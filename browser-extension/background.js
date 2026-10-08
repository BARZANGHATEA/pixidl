// Background script (Chromium service worker / Firefox event page).
// Owns the context menus and the optional capture of browser downloads.
// Listeners are registered synchronously at top level so the browser can wake
// the worker for them.

import { api, errorText, send, sendMany } from "./native.js";
import { collectSelectionLinks } from "./collect.js";
import { loadSettings } from "./settings.js";
import { basename, buildItem, isHttpUrl, meetsMinSize, normalizeLinks, supportedUrl } from "./lib.js";

const t = (key, subs) => api.i18n.getMessage(key, subs);

const MENU = Object.freeze({
  link: "pixidl-download-link",
  media: "pixidl-download-media",
  page: "pixidl-send-page",
  selection: "pixidl-download-selection",
});

async function setupMenus() {
  await api.contextMenus.removeAll();
  const items = [
    { id: MENU.link, title: t("menuDownloadLink"), contexts: ["link"] },
    { id: MENU.media, title: t("menuDownloadMedia"), contexts: ["video", "audio", "image"] },
    { id: MENU.page, title: t("menuSendPage"), contexts: ["page"] },
    { id: MENU.selection, title: t("menuDownloadSelection"), contexts: ["selection"] },
  ];
  for (const item of items) api.contextMenus.create(item);
}

api.runtime.onInstalled.addListener(setupMenus);
api.runtime.onStartup.addListener(setupMenus);

/** Shows a basic system notification; failures to notify are not fatal. */
function notify(ok, message) {
  try {
    Promise.resolve(
      api.notifications.create({
        type: "basic",
        iconUrl: api.runtime.getURL("icons/icon-128.png"),
        title: t(ok ? "notifySentTitle" : "notifyFailedTitle"),
        message: String(message || ""),
      }),
    ).catch(() => {});
  } catch {
    // Notifications may be disabled at the OS level.
  }
}

/** Sends a single URL with add_download and reports the outcome. */
async function addOne(rawUrl, referrer) {
  const url = supportedUrl(rawUrl);
  if (!url) return notify(false, t("errInvalidUrl"));
  const resp = await send("add_download", buildItem(url, { referrer }));
  notify(resp.success, resp.success ? resp.filename || url : errorText(resp.error));
}

/** Collects the links inside the selection and sends them as one or more batches. */
async function addSelection(tab, info) {
  let raw;
  try {
    raw = await collectSelectionLinks(tab.id, info.frameId);
  } catch {
    return notify(false, t("errCannotAccessPage"));
  }
  const urls = normalizeLinks(raw);
  if (urls.length === 0) return notify(false, t("noLinksInSelection"));
  const referrer = info.frameUrl || info.pageUrl || tab.url;
  const { added, total, error } = await sendMany(urls.map((url) => buildItem(url, { referrer })));
  if (added > 0) notify(true, t("addedCount", [String(added), String(total)]));
  else notify(false, error ? errorText(error) : t("noneAdded"));
}

api.contextMenus.onClicked.addListener((info, tab) => {
  const referrer = info.frameUrl || info.pageUrl;
  switch (info.menuItemId) {
    case MENU.link:
      return addOne(info.linkUrl, referrer);
    case MENU.media:
      return addOne(info.srcUrl, referrer);
    case MENU.page:
      return addOne(info.pageUrl || tab?.url);
    case MENU.selection:
      return tab?.id != null ? addSelection(tab, info) : undefined;
    default:
      return undefined;
  }
});

// Optional capture of browser downloads. pixidl is asked first; the browser
// download is cancelled only after pixidl accepted it, so any failure leaves the
// browser to finish the download as usual.
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

// Toolbar popup: connection status, page actions, active downloads, options.

import { api, errorText, send, sendMany } from "./native.js";
import { collectPageLinks } from "./collect.js";
import { loadSettings, saveSettings } from "./settings.js";
import { buildItem, formatBytes, matchesExtensions, normalizeLinks, parseExtensionFilter, percent, supportedUrl } from "./lib.js";

const POLL_MS = 2000;
const t = (key, subs) => api.i18n.getMessage(key, subs) || key;
const $ = (id) => document.getElementById(id);

let tab = null; // the active tab (activeTab grants its URL while the popup is open)
let links = []; // validated, de-duplicated links collected from the page
let pollTimer = 0;
let refreshing = null; // the in-flight get_status request, so polls never overlap

function localize() {
  document.documentElement.dir = api.i18n.getMessage("@@bidi_dir") || "ltr";
  document.documentElement.lang = api.i18n.getUILanguage();
  for (const el of document.querySelectorAll("[data-i18n]")) el.textContent = t(el.dataset.i18n);
  for (const el of document.querySelectorAll("[data-i18n-placeholder]")) el.placeholder = t(el.dataset.i18nPlaceholder);
}

function showMessage(text, kind = "info") {
  const el = $("message");
  el.textContent = text;
  el.dataset.kind = kind;
  el.hidden = !text;
}

function setConnection(state, text) {
  $("conn").dataset.state = state;
  $("conn-text").textContent = text;
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

// ---- Active downloads -------------------------------------------------------

function controlButton(action, download) {
  const button = el("button", "small", t(action));
  button.type = "button";
  button.addEventListener("click", async () => {
    button.disabled = true;
    const resp = await send(action, { download_id: download.download_id });
    if (!resp.success) showMessage(errorText(resp.error), "error");
    await refreshDownloads();
  });
  return button;
}

function downloadRow(d) {
  const row = el("li", "item");
  const name = el("div", "name", d.filename || d.download_id);
  name.title = name.textContent;

  const pct = percent(d.downloaded_bytes, d.total_bytes);
  const bar = el("div", pct === null ? "bar unknown" : "bar");
  const fill = el("div", "fill");
  fill.style.width = `${pct ?? 0}%`;
  bar.append(fill);

  const status = api.i18n.getMessage(`status_${d.status}`) || String(d.status);
  const amount = pct !== null ? `${pct}%` : formatBytes(d.downloaded_bytes);
  const meta = el("div", "meta");
  meta.append(el("span", "", [status, amount].filter(Boolean).join(" · ")));

  const controls = el("span", "controls");
  controls.append(controlButton(d.status === "paused" ? "resume" : "pause", d), controlButton("cancel", d));
  meta.append(controls);

  row.append(name, bar, meta);
  return row;
}

function refreshDownloads() {
  refreshing ??= loadDownloads().finally(() => {
    refreshing = null;
  });
  return refreshing;
}

async function loadDownloads() {
  clearTimeout(pollTimer);
  const resp = await send("get_status");
  const list = resp.success && Array.isArray(resp.downloads) ? resp.downloads : [];
  $("downloads").replaceChildren(...list.map(downloadRow));
  $("no-downloads").hidden = list.length > 0;
  if (!resp.success) {
    setConnection("error", errorText(resp.error));
    if (resp.error.code === "app_unavailable") return; // stop polling until reopened
  }
  pollTimer = setTimeout(refreshDownloads, POLL_MS);
}

async function checkConnection() {
  const resp = await send("ping");
  if (!resp.success) {
    setConnection("error", errorText(resp.error));
    return;
  }
  setConnection("ok", t("statusConnected", [String(resp.app_version ?? "")]));
  await refreshDownloads();
}

// ---- Page actions -----------------------------------------------------------

async function sendPage() {
  const url = supportedUrl(tab?.url);
  if (!url) return showMessage(t("errInvalidUrl"), "error");
  showMessage(t("sending"));
  const resp = await send("add_download", buildItem(url));
  if (!resp.success) return showMessage(errorText(resp.error), "error");
  showMessage(t("sentOne", [resp.filename || url]), "ok");
  await refreshDownloads();
}

function filteredLinks() {
  const extensions = parseExtensionFilter($("filter").value);
  return links.filter((url) => matchesExtensions(url, extensions));
}

function updateLinkCount() {
  const count = filteredLinks().length;
  $("links-count").textContent = t("linksFound", [String(count), String(links.length)]);
  $("send-links").textContent = t("sendLinks", [String(count)]);
  $("send-links").disabled = count === 0;
}

async function collectLinks() {
  if (tab?.id == null) return showMessage(t("errCannotAccessPage"), "error");
  try {
    links = normalizeLinks(await collectPageLinks(tab.id), { exclude: [tab.url] });
  } catch {
    links = [];
    $("links").hidden = true;
    return showMessage(t("errCannotAccessPage"), "error");
  }
  showMessage("");
  $("links").hidden = false;
  updateLinkCount();
  $("filter").focus();
}

async function sendLinks() {
  const urls = filteredLinks();
  if (urls.length === 0) return;
  $("send-links").disabled = true;
  showMessage(t("sending"));
  const { added, total, error } = await sendMany(urls.map((url) => buildItem(url, { referrer: tab?.url })));
  if (added > 0) showMessage(t("addedCount", [String(added), String(total)]), added === total ? "ok" : "error");
  else showMessage(error ? errorText(error) : t("noneAdded"), "error");
  updateLinkCount();
  await refreshDownloads();
}

// ---- Options ----------------------------------------------------------------

async function initSettings() {
  const settings = await loadSettings();
  $("capture").checked = settings.captureDownloads;
  $("min-size").value = String(settings.minSizeMb);
  $("capture").addEventListener("change", () => saveSettings({ captureDownloads: $("capture").checked }));
  $("min-size").addEventListener("change", async () => {
    const saved = await saveSettings({ minSizeMb: $("min-size").value });
    $("min-size").value = String(saved.minSizeMb);
  });
}

async function main() {
  localize();
  [tab] = await api.tabs.query({ active: true, currentWindow: true });
  $("send-page").addEventListener("click", sendPage);
  $("collect").addEventListener("click", collectLinks);
  $("send-links").addEventListener("click", sendLinks);
  $("filter").addEventListener("input", updateLinkCount);
  await initSettings();
  await checkConnection();
}

main();

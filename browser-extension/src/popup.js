// Toolbar popup: connection status, detected downloads, page actions and the
// list of active downloads in pixidl.

import { api, cachedPing, errorText, localizePage, pingAndCache, send, t } from "./native.js";
import { applyAccent, loadSettings } from "./settings.js";
import { icon } from "./icons.js";
import { buildItem, formatBytes, isHttpUrl, percent, supportedUrl } from "./lib.js";

const POLL_MS = 2000;
const ACTIVE = new Set(["queued", "preparing", "downloading", "paused"]);
const $ = (id) => document.getElementById(id);

let tab = null; // the active tab (activeTab grants its URL while the popup is open)
let pollTimer = 0;
let refreshing = null; // the in-flight get_status request, so polls never overlap

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function showMessage(text, kind = "info") {
  const box = $("message");
  box.textContent = text;
  box.dataset.kind = kind;
  box.hidden = !text;
}

// ---- Connection ------------------------------------------------------------------

function showConnection(ping) {
  const conn = $("conn");
  if (!ping) {
    conn.dataset.state = "checking";
    $("conn-text").textContent = t("statusChecking");
  } else if (ping.success === false || ping.connected === false) {
    conn.dataset.state = "error";
    $("conn-text").textContent = ping.error ? errorText(ping.error) : t("errAppUnavailable");
  } else if (ping.integration_enabled === false) {
    conn.dataset.state = "warn";
    $("conn-text").textContent = t("errUnauthorized");
  } else {
    conn.dataset.state = "ok";
    $("conn-text").textContent = t("statusConnected", [String(ping.app_version ?? "")]);
  }
}

async function checkConnection() {
  const cached = await cachedPing();
  if (cached?.connected) showConnection(cached);
  const resp = await pingAndCache();
  showConnection(resp);
  await applyAccent();
  return resp.success;
}

// ---- Detected downloads on this page ------------------------------------------------

async function showDetected() {
  const settings = await loadSettings();
  let count = 0;
  if (settings.detectLinks && tab?.id != null) {
    try {
      const detected = await api.tabs.sendMessage(tab.id, { type: "pixidl:getDetected" }, { frameId: 0 });
      count = Math.max(0, Number(detected?.count) || 0);
    } catch {
      count = 0; // no content script here (browser page, store, PDF viewer...)
    }
  }
  $("detected-count").textContent = count === 1 ? t("detectedOne") : t("detectedMany", [String(count)]);
  $("detected-hint").textContent = settings.detectLinks ? t("detectedHint") : t("detectionOff");
  $("review").disabled = count === 0;
}

async function openFromBackground(type) {
  if (tab?.id == null) return;
  // The background opens the picker window (or shows why it cannot) and
  // outlives the popup, which closes once the request is on its way.
  const sent = api.runtime.sendMessage({ type, tabId: tab.id }).catch(() => {});
  await Promise.race([sent, new Promise((resolve) => setTimeout(resolve, 150))]);
  window.close();
}

async function sendPage() {
  const url = supportedUrl(tab?.url);
  if (!url || !isHttpUrl(url)) return showMessage(t("errInvalidUrl"), "error");
  $("send-page").disabled = true;
  showMessage(t("sending"));
  const payload = buildItem(url);
  const resp = await send("open_in_app", payload);
  $("send-page").disabled = false;
  if (!resp.success) return showMessage(errorText(resp.error), "error");
  showMessage(t("openedInApp"), "ok");
}

// ---- Active downloads -----------------------------------------------------------------

function controlButton(action, download) {
  const button = el("button", "icon-btn");
  button.type = "button";
  button.title = t(action);
  button.setAttribute("aria-label", `${t(action)}: ${download.filename || ""}`);
  button.append(icon(action, 15));
  button.addEventListener("click", async () => {
    button.disabled = true;
    const resp = await send(action, { download_id: download.download_id });
    if (!resp.success) showMessage(errorText(resp.error), "error");
    await refreshDownloads();
  });
  return button;
}

function downloadRow(d) {
  const row = el("li", "dl");
  const top = el("div", "dl-top");
  const name = el("div", "dl-name", d.filename || String(d.download_id));
  name.title = name.textContent;
  const controls = el("div", "dl-controls");
  if (d.status === "paused") controls.append(controlButton("resume", d));
  else if (d.status === "downloading" || d.status === "queued" || d.status === "preparing") controls.append(controlButton("pause", d));
  controls.append(controlButton("cancel", d));
  top.append(name, controls);

  const pct = percent(d.downloaded_bytes, d.total_bytes);
  const bar = el("div", pct === null ? "bar unknown" : "bar");
  bar.setAttribute("role", "progressbar");
  if (pct !== null) bar.setAttribute("aria-valuenow", String(pct));
  const fill = el("div", "fill");
  fill.style.width = `${pct ?? 0}%`;
  bar.append(fill);

  const status = api.i18n.getMessage(`status_${d.status}`) || String(d.status);
  const done = formatBytes(d.downloaded_bytes);
  const total = formatBytes(d.total_bytes);
  const amount = total ? `${done} / ${total}` : done;
  const speed = d.status === "downloading" && d.speed_bytes_per_second > 0 ? `${formatBytes(d.speed_bytes_per_second)}/s` : "";
  const meta = el("div", "dl-meta", [status, pct !== null ? `${pct}%` : "", amount, speed].filter(Boolean).join(" · "));
  row.append(top, bar, meta);
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
  const list = resp.success && Array.isArray(resp.downloads) ? resp.downloads.filter((d) => ACTIVE.has(d.status)) : [];
  $("downloads").replaceChildren(...list.map(downloadRow));
  $("no-downloads").hidden = list.length > 0;
  if (!resp.success) {
    showConnection(resp);
    if (resp.error.code === "app_unavailable" || resp.error.code === "unauthorized") return; // stop polling
  }
  pollTimer = setTimeout(refreshDownloads, POLL_MS);
}

// ---- Startup ---------------------------------------------------------------------

async function main() {
  localizePage();
  applyAccent();
  [tab] = await api.tabs.query({ active: true, currentWindow: true });
  const pageOk = isHttpUrl(tab?.url);
  $("all-links").disabled = !pageOk;
  $("send-page").disabled = !pageOk;
  $("review").addEventListener("click", () => openFromBackground("pixidl:reviewDetected"));
  $("all-links").addEventListener("click", () => openFromBackground("pixidl:allLinks"));
  $("send-page").addEventListener("click", sendPage);
  $("open-settings").addEventListener("click", () => {
    api.runtime.openOptionsPage();
    window.close();
  });
  showDetected();
  if (await checkConnection()) await refreshDownloads();
  else $("no-downloads").hidden = false;
}

main();

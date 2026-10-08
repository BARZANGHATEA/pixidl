// Picker window: review a list of links, see their real names and sizes (from
// probe_links), filter them and send the chosen ones to pixidl.

import { api, errorText, localizePage, probeAll, sendMany, t } from "./native.js";
import { applyAccent } from "./settings.js";
import { TYPE_COLORS, icon, typeBadge } from "./icons.js";
import {
  PROBE_CHUNK,
  buildItem,
  countByType,
  displayName,
  fileTypeOf,
  filterItems,
  formatBytes,
  hostOf,
  normalizeEntries,
  totalSize,
} from "./lib.js";

const $ = (id) => document.getElementById(id);
const sessionArea = () => api.storage.session ?? api.storage.local;

let items = []; // {index, url, label, name, host, type, size, probing, error, selected, row}
let visible = []; // items currently shown (search + type chips)
let pageUrl = "";
let query = "";
const activeTypes = new Set();
let sending = false;
let focusIndex = -1; // index into `visible` of the row that owns the roving tabindex

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

// ---- Loading the link list ---------------------------------------------------------

async function loadSession() {
  const id = new URLSearchParams(location.search).get("s");
  if (!id || !/^[0-9a-f-]{36}$/i.test(id)) return null;
  const key = `picker:${id}`;
  try {
    const data = (await sessionArea().get(key))[key] ?? null;
    await sessionArea().remove(key);
    // Remove the id from the address so a reload does not look for it again.
    history.replaceState(null, "", location.pathname);
    return data;
  } catch {
    return null;
  }
}

// ---- Rendering ---------------------------------------------------------------------

function sizeCell(item) {
  const cell = item.row.querySelector(".size");
  cell.classList.toggle("loading", item.probing);
  cell.textContent = item.probing ? "…" : formatBytes(item.size);
}

function updateRow(item) {
  const { row } = item;
  row.querySelector(".name").textContent = item.name;
  const badge = row.querySelector(".ftype");
  if (badge.dataset.type !== item.type) badge.replaceWith(typeBadge(item.type));
  const err = row.querySelector(".err");
  err.textContent = item.error ? t("probeFailed") : "";
  err.title = item.error;
  sizeCell(item);
}

function setRowSelected(item, selected) {
  item.selected = selected;
  item.row.setAttribute("aria-selected", String(selected));
  item.row.querySelector("input").checked = selected;
}

function buildRow(item) {
  const row = el("li", "row");
  row.setAttribute("role", "option");
  row.tabIndex = -1;
  row.dataset.index = String(item.index);
  row.title = item.url;
  const box = el("input");
  box.type = "checkbox";
  box.tabIndex = -1;
  box.setAttribute("aria-hidden", "true");
  const main = el("div", "main");
  const name = el("div", "name");
  name.dir = "auto";
  const sub = el("div", "sub");
  const host = el("span", "host", item.host || (item.url.startsWith("magnet:") ? "magnet" : ""));
  host.dir = "ltr";
  sub.append(host, el("span", "err"));
  main.append(name, sub);
  row.append(box, typeBadge(item.type), main, el("div", "size"));
  item.row = row;
  setRowSelected(item, item.selected);
  updateRow(item);
  return row;
}

function renderChips() {
  const counts = countByType(items);
  for (const type of [...activeTypes]) if (!counts[type]) activeTypes.delete(type);
  const chips = Object.entries(counts).map(([type, n]) => {
    const chip = el("button", "chip");
    chip.type = "button";
    chip.setAttribute("aria-pressed", String(activeTypes.has(type)));
    chip.style.setProperty("--type-color", TYPE_COLORS[type]);
    chip.append(el("span", "swatch"), el("span", "", t(`type_${type}`)), el("span", "n", String(n)));
    chip.addEventListener("click", () => {
      if (activeTypes.has(type)) activeTypes.delete(type);
      else activeTypes.add(type);
      chip.setAttribute("aria-pressed", String(activeTypes.has(type)));
      applyFilter();
    });
    return chip;
  });
  $("chips").replaceChildren(...chips);
}

function applyFilter() {
  visible = filterItems(items, { query, types: [...activeTypes] });
  const shown = new Set(visible);
  for (const item of items) item.row.hidden = !shown.has(item);
  $("empty").hidden = visible.length > 0;
  $("empty").textContent = items.length ? t("noMatches") : t("noLinks");
  if (focusIndex >= visible.length) focusIndex = visible.length - 1;
  setRovingFocus(Math.max(0, focusIndex), false);
  updateSummary();
}

function updateSummary() {
  const selected = visible.filter((item) => item.selected);
  const { bytes, unknown } = totalSize(selected);
  let size = formatBytes(bytes);
  if (unknown > 0 && selected.length > unknown) size = t("sizeKnownOnly", [size]);
  else if (unknown > 0) size = "";
  $("summary").textContent = size ? t("selectedWithSize", [String(selected.length), size]) : t("selectedCount", [String(selected.length)]);
  $("send").disabled = selected.length === 0 || sending;
}

function setStatus(text, kind = "") {
  $("status").textContent = text;
  $("status").dataset.kind = kind;
}

function showBanner(text) {
  const banner = $("banner");
  banner.replaceChildren(icon("alert", 16), el("span", "", text));
  banner.hidden = false;
}

// ---- Selection and keyboard ---------------------------------------------------------

function setRovingFocus(index, focus = true) {
  for (const item of items) item.row.tabIndex = -1;
  const item = visible[index];
  if (!item) {
    focusIndex = -1;
    return;
  }
  focusIndex = index;
  item.row.tabIndex = 0;
  if (focus) {
    item.row.focus();
    item.row.scrollIntoView({ block: "nearest" });
  }
}

function toggle(item) {
  setRowSelected(item, !item.selected);
  updateSummary();
}

function selectVisible(selected) {
  for (const item of visible) setRowSelected(item, selected);
  updateSummary();
}

function onListClick(event) {
  const row = event.target.closest(".row");
  if (!row) return;
  const item = items[Number(row.dataset.index)];
  toggle(item);
  setRovingFocus(visible.indexOf(item));
}

function onKeyDown(event) {
  const target = event.target;
  const inList = target.classList?.contains("row");
  if (event.key === "Escape") {
    event.preventDefault();
    window.close();
    return;
  }
  if (event.key === "Enter") {
    if (target.tagName === "BUTTON") return; // the focused button handles it
    event.preventDefault();
    send();
    return;
  }
  if (inList && event.key === " ") {
    event.preventDefault();
    toggle(items[Number(target.dataset.index)]);
    return;
  }
  const moves = { ArrowDown: 1, ArrowUp: -1, Home: -Infinity, End: Infinity };
  if (!(event.key in moves) || visible.length === 0) return;
  if (inList) {
    event.preventDefault();
    const step = moves[event.key];
    const next = Number.isFinite(step) ? focusIndex + step : step < 0 ? 0 : visible.length - 1;
    setRovingFocus(Math.min(Math.max(0, next), visible.length - 1));
  } else if (target.id === "search" && event.key === "ArrowDown") {
    event.preventDefault();
    setRovingFocus(Math.max(0, focusIndex));
  }
}

// ---- Probing names and sizes --------------------------------------------------------

async function probe() {
  const requests = items.map((item) => buildItem(item.url, { referrer: pageUrl }));
  const error = await probeAll(requests, (start, results, chunkError) => {
    const count = chunkError ? Math.min(PROBE_CHUNK, items.length - start) : results.length;
    for (let i = 0; i < count; i++) {
      const item = items[start + i];
      if (!item) continue;
      const result = chunkError ? null : results[i];
      item.probing = false;
      if (result && typeof result === "object") {
        if (typeof result.filename === "string" && result.filename.trim()) item.name = result.filename.trim().slice(0, 255);
        item.size = Number.isFinite(result.totalBytes) && result.totalBytes >= 0 ? result.totalBytes : null;
        item.type = fileTypeOf({ url: item.url, filename: result.filename || "", contentType: result.contentType || "", engine: result.engine || "" });
        item.error = typeof result.error === "string" ? result.error : "";
      }
      updateRow(item);
    }
    renderChips();
    applyFilter();
  });
  // Rows left over after a fatal error stop "loading".
  for (const item of items) {
    if (item.probing) {
      item.probing = false;
      updateRow(item);
    }
  }
  if (error) {
    if (error.code === "app_unavailable") showBanner(t("bannerNotRunning"));
    else if (error.code === "unauthorized") showBanner(t("errUnauthorized"));
    else showBanner(t("bannerProbeFailed", [errorText(error)]));
  }
}

// ---- Sending ---------------------------------------------------------------------

async function send() {
  const chosen = visible.filter((item) => item.selected);
  if (sending || chosen.length === 0) return;
  sending = true;
  $("send").disabled = true;
  $("cancel").disabled = true;
  setStatus(t("sending"));
  // Only url + referrer: the app works out the file name itself.
  const { added, total, error } = await sendMany(chosen.map((item) => buildItem(item.url, { referrer: pageUrl })));
  sending = false;
  $("cancel").disabled = false;
  if (added === total && total > 0 && !error) {
    setStatus(t("sentCount", [String(added), String(total)]), "ok");
    setTimeout(() => window.close(), 1200);
    return;
  }
  const parts = [];
  if (added > 0) parts.push(t("sentCount", [String(added), String(total)]));
  parts.push(error ? errorText(error) : t("noneAdded"));
  setStatus(parts.join(" — "), added > 0 && added === total ? "ok" : "error");
  updateSummary();
}

// ---- Startup ---------------------------------------------------------------------

async function main() {
  localizePage();
  applyAccent();
  $("search-icon").append(icon("search", 16));
  const data = await loadSession();
  pageUrl = typeof data?.pageUrl === "string" ? data.pageUrl : "";
  const links = normalizeEntries(data?.links);
  items = links.map((entry, index) => ({
    index,
    url: entry.url,
    label: entry.label,
    name: displayName(entry.url, entry.label),
    host: hostOf(entry.url),
    type: fileTypeOf({ url: entry.url }),
    size: null,
    probing: true,
    error: "",
    selected: true,
    row: null,
  }));

  const title = items.length === 1 ? t("pickerLinksOne") : t("pickerLinksMany", [String(items.length)]);
  $("title").textContent = title;
  document.title = `pixidl — ${title}`;
  $("source").textContent = hostOf(pageUrl) || (typeof data?.title === "string" ? data.title : "");
  $("source").title = pageUrl;

  const fragment = document.createDocumentFragment();
  for (const item of items) fragment.append(buildRow(item));
  $("list").append(fragment);
  renderChips();
  applyFilter();

  $("list").addEventListener("click", onListClick);
  $("search").addEventListener("input", () => {
    query = $("search").value;
    applyFilter();
  });
  $("select-all").addEventListener("click", () => selectVisible(true));
  $("select-none").addEventListener("click", () => selectVisible(false));
  $("cancel").addEventListener("click", () => window.close());
  $("send").addEventListener("click", send);
  document.addEventListener("keydown", onKeyDown);

  if (!data) {
    setStatus(t("pickerExpired"), "error");
    return;
  }
  (visible.length ? visible[0].row : $("search")).focus();
  if (items.length) probe();
}

main();

// Pure helpers shared by every part of the extension.
//
// Rules for this file (build.mjs relies on them):
// - no imports and no browser globals touched at import time, so it runs in
//   Node for the tests;
// - only `export function` / `export const` declarations, so the build can
//   strip the `export` keywords and inline it into the classic content script.

export const HOST_NAME = "com.pixidl.app";
export const PROTOCOL_VERSION = 1;
/** Maximum items per add_multiple_downloads / probe_links message (enforced by the app). */
export const MAX_BATCH = 200;
/**
 * Byte budget for one batch request. The host caps messages at 1 MiB in each
 * direction and the response echoes every URL, so stay well below that.
 */
export const MAX_BATCH_BYTES = 512 * 1024;
/** Links per probe_links request sent by the picker. */
export const PROBE_CHUNK = 50;
/** Same limit as the app's URL validator. */
export const MAX_URL_LEN = 16 * 1024;
/** Upper bound on automatically detected download links per page. */
export const MAX_DETECTED = 1000;
/** Upper bound on links collected by "Download all links on this page". */
export const MAX_PAGE_LINKS = 5000;
/** Longest label kept for a link (its text on the page). */
export const MAX_LABEL = 200;
export const DEFAULT_ACCENT = "#2563EB";

// ---- Protocol envelopes ------------------------------------------------------

/** A correlation id for one request (at most 128 chars per the protocol). */
export function makeId() {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`;
}

/** Builds a protocol request envelope; `client` is `{browser, version}` when known. */
export function buildEnvelope(type, payload = {}, id = makeId(), client = null) {
  const envelope = { version: PROTOCOL_VERSION, type, id, payload };
  if (client && typeof client.browser === "string" && typeof client.version === "string") {
    envelope.client = { browser: client.browser, version: client.version };
  }
  return envelope;
}

/** A failure response in the same shape the app uses. */
export function errorResponse(code, message, id = null) {
  return { version: PROTOCOL_VERSION, id, success: false, error: { code, message } };
}

/** Guarantees the caller always gets `{success, ...}` with an `error` on failure. */
export function normalizeResponse(resp, id = null) {
  if (!resp || typeof resp !== "object" || Array.isArray(resp)) {
    return errorResponse("internal", "Invalid response from the native host", id);
  }
  if (resp.success === true) return resp;
  const error = resp.error && typeof resp.error === "object" ? resp.error : {};
  return {
    ...resp,
    success: false,
    error: { code: String(error.code || "internal"), message: String(error.message || "Unknown error") },
  };
}

// ---- URL validation ------------------------------------------------------------

/**
 * Returns the normalized URL string if pixidl can download it, otherwise null.
 * Accepted: http(s) with a host (fragment removed), and magnet links carrying
 * an info hash. ftp, file, data, blob, javascript... are never sent.
 */
export function supportedUrl(input) {
  if (typeof input !== "string") return null;
  const raw = input.trim();
  if (!raw || raw.length > MAX_URL_LEN) return null;
  let url;
  try {
    url = new URL(raw);
  } catch {
    return null;
  }
  if (url.protocol === "http:" || url.protocol === "https:") {
    if (!url.hostname || url.username || url.password) return null;
    url.hash = "";
    return url.href;
  }
  if (url.protocol === "magnet:") {
    const params = new URLSearchParams(url.search);
    const hasHash = params.getAll("xt").some((xt) => /^urn:bt(ih|mh):[0-9a-z]+$/i.test(xt));
    return hasHash ? raw.split("#")[0] : null;
  }
  return null;
}

/** True for plain http(s) URLs. */
export function isHttpUrl(input) {
  const url = supportedUrl(input);
  return url !== null && /^https?:/i.test(url);
}

/** A referrer is only forwarded when it is an http(s) page. */
export function safeReferrer(input) {
  return isHttpUrl(input) ? supportedUrl(input) : undefined;
}

/** Host name of an http(s) URL ("" for magnets and invalid input). */
export function hostOf(input) {
  try {
    const url = new URL(input);
    return url.protocol === "http:" || url.protocol === "https:" ? url.hostname : "";
  } catch {
    return "";
  }
}

/** Accent colors are only used when they are plain `#RRGGBB`. */
export function safeAccent(color) {
  return typeof color === "string" && /^#[0-9a-f]{6}$/i.test(color) ? color : DEFAULT_ACCENT;
}

// ---- Link lists ------------------------------------------------------------------

function cleanLabel(label) {
  return typeof label === "string" ? label.replace(/\s+/g, " ").trim().slice(0, MAX_LABEL) : "";
}

/**
 * Validates, normalizes (fragment removed) and de-duplicates links, keeping the
 * first-seen order. Entries are URL strings or `{url, label}`; the result is
 * always `{url, label}`. A later duplicate can supply a missing label.
 * `exclude` drops e.g. the page's own URL.
 */
export function normalizeEntries(entries, { exclude = [], limit = MAX_PAGE_LINKS } = {}) {
  const skip = new Set(exclude.map(supportedUrl).filter(Boolean));
  const byUrl = new Map();
  for (const entry of Array.isArray(entries) ? entries : []) {
    const raw = typeof entry === "string" ? entry : entry?.url;
    const url = supportedUrl(raw);
    if (!url || skip.has(url)) continue;
    const label = cleanLabel(typeof entry === "string" ? "" : entry?.label);
    const existing = byUrl.get(url);
    if (existing) {
      if (!existing.label && label) existing.label = label;
      continue;
    }
    if (byUrl.size >= limit) continue;
    byUrl.set(url, { url, label });
  }
  return [...byUrl.values()];
}

/** Like normalizeEntries but returns plain URL strings. */
export function normalizeLinks(urls, options) {
  return normalizeEntries(urls, options).map((e) => e.url);
}

/**
 * Finds http(s) and magnet URLs typed in plain text (e.g. the selected text),
 * plus addresses written without a scheme that start with `www.` (read as
 * https). Trailing punctuation that usually ends a sentence is not part of
 * the URL. ftp:// and other schemes are ignored: pixidl does not take them.
 */
export function extractUrlsFromText(text, limit = MAX_PAGE_LINKS) {
  if (typeof text !== "string" || !text) return [];
  const out = [];
  const re = /(?:https?:\/\/|magnet:\?|(?<![\p{L}\p{N}_.@/:-])www\.(?=[\p{L}\p{N}-]+\.[\p{L}\p{N}]))[^\s<>"'`«»“”]+/giu;
  for (const match of text.matchAll(re)) {
    let url = /^www\./i.test(match[0]) ? `https://${match[0]}` : match[0];
    for (;;) {
      const trimmed = url.replace(/[.,;:!?'"*،؛۔、。]+$/u, "");
      let next = trimmed;
      for (const [open, close] of [["(", ")"], ["[", "]"], ["{", "}"]]) {
        if (next.endsWith(close) && next.split(open).length < next.split(close).length) next = next.slice(0, -1);
      }
      if (next === url) break;
      url = next;
    }
    if (supportedUrl(url)) out.push(url);
    if (out.length >= limit) break;
  }
  return out;
}

/** Splits a list into consecutive chunks of at most `size` items. */
export function chunk(list, size) {
  const n = Math.max(1, Math.floor(size) || 1);
  const out = [];
  for (let i = 0; i < list.length; i += n) out.push(list.slice(i, i + n));
  return out;
}

/**
 * Splits items into batches of at most `maxCount` entries whose JSON size
 * stays under `maxBytes` (a single oversized item still gets its own batch).
 */
export function batchItems(items, maxCount = MAX_BATCH, maxBytes = MAX_BATCH_BYTES) {
  const batches = [];
  let current = [];
  let size = 0;
  for (const item of items) {
    const itemSize = new TextEncoder().encode(JSON.stringify(item)).length + 1;
    if (current.length > 0 && (current.length >= maxCount || size + itemSize > maxBytes)) {
      batches.push(current);
      current = [];
      size = 0;
    }
    current.push(item);
    size += itemSize;
  }
  if (current.length > 0) batches.push(current);
  return batches;
}

// ---- File names and types --------------------------------------------------------

/** File name part of a local path (either separator); "" when there is none. */
export function basename(path) {
  if (typeof path !== "string") return "";
  return path.split(/[\\/]/).pop().trim();
}

/** Last path segment of an http(s) URL, or the `dn` of a magnet link, decoded. */
export function urlFileName(input) {
  let url;
  try {
    url = new URL(input);
  } catch {
    return "";
  }
  if (url.protocol === "magnet:") return new URLSearchParams(url.search).get("dn")?.trim() || "";
  const segment = url.pathname.split("/").pop() || "";
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}

/** Lower-case extension of a file name or URL path ("" when there is none). */
export function fileExtension(nameOrUrl) {
  let name = String(nameOrUrl ?? "");
  if (/^[a-z][a-z0-9+.-]*:/i.test(name)) name = urlFileName(name);
  name = basename(name);
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return "";
  const ext = name.slice(dot + 1).toLowerCase();
  return /^[a-z0-9]{1,10}$/.test(ext) ? ext : "";
}

export const TYPE_ORDER = Object.freeze(["video", "audio", "archive", "program", "document", "image", "torrent", "other"]);

const EXT_TYPES = (() => {
  const map = new Map();
  const add = (type, list) => list.split(" ").forEach((ext) => map.set(ext, type));
  add("video", "mp4 mkv webm avi mov m4v wmv flv mpg mpeg 3gp");
  add("audio", "mp3 flac wav m4a ogg opus aac wma");
  add("archive", "zip rar 7z tar gz tgz bz2 xz zst iso img");
  add("program", "exe msi msix dmg pkg apk deb rpm appimage");
  add("document", "pdf doc docx xls xlsx ppt pptx epub csv odt ods odp rtf txt");
  add("image", "jpg jpeg png gif webp avif bmp svg");
  add("torrent", "torrent");
  return map;
})();

/** Extensions that make a link count as a download when found on a page. */
export const DOWNLOAD_EXTENSIONS = Object.freeze(
  "zip rar 7z tar gz tgz bz2 xz zst iso img dmg exe msi msix apk deb rpm appimage pdf doc docx xls xlsx ppt pptx epub csv mp4 mkv webm avi mov m4v mp3 flac wav m4a ogg opus aac torrent".split(" "),
);
/** Image extensions only count when an `<a href>` points straight at the image. */
export const LINKED_IMAGE_EXTENSIONS = Object.freeze(["jpg", "jpeg", "png", "gif", "webp"]);

/**
 * Picker file type for a link: "video", "audio", "archive", "program",
 * "document", "image", "torrent" or "other". The probed file name and content
 * type win over the URL, and the app's engine choice wins over both.
 */
export function fileTypeOf({ url = "", filename = "", contentType = "", engine = "" } = {}) {
  if (engine === "torrent" || /^magnet:/i.test(url)) return "torrent";
  if (engine === "video") return "video";
  for (const source of [filename, url]) {
    const type = EXT_TYPES.get(fileExtension(source));
    if (type) return type;
  }
  const mime = String(contentType || "").toLowerCase().split(";")[0].trim();
  if (mime === "application/x-bittorrent") return "torrent";
  if (mime.startsWith("video/")) return "video";
  if (mime.startsWith("audio/")) return "audio";
  if (mime.startsWith("image/")) return "image";
  if (/^application\/(zip|x-zip-compressed|x-rar|vnd\.rar|x-7z-compressed|gzip|x-tar|x-xz|zstd|x-iso9660-image)/.test(mime)) return "archive";
  if (/^application\/(x-msdownload|x-msi|vnd\.android\.package-archive|x-apple-diskimage|vnd\.debian\.binary-package|x-rpm)/.test(mime)) return "program";
  if (mime === "application/pdf" || mime.startsWith("text/csv") || /^application\/(msword|vnd\.openxmlformats|vnd\.ms-|epub)/.test(mime)) return "document";
  return "other";
}

/**
 * Whether a link found on a page counts as a download for automatic detection.
 * `source` is "anchor" (an `<a href>`) or "media" (video/audio/source src).
 */
export function isDownloadLink(href, { source = "anchor", hasDownloadAttr = false } = {}) {
  const url = supportedUrl(href);
  if (!url) return false;
  if (url.startsWith("magnet:")) return true;
  if (source === "anchor" && hasDownloadAttr) return true;
  const ext = fileExtension(url);
  if (DOWNLOAD_EXTENSIONS.includes(ext)) return true;
  return source === "anchor" && LINKED_IMAGE_EXTENSIONS.includes(ext);
}

/** Name to show for a link before (or without) a probe result. */
export function displayName(url, label = "") {
  return urlFileName(url) || hostOf(url) || cleanLabel(label) || String(url).slice(0, 80);
}

// ---- Picker list -------------------------------------------------------------

/**
 * Items whose name or URL contains every word of `query` (case-insensitive)
 * and whose `type` is one of `types` (an empty set means all types).
 */
export function filterItems(items, { query = "", types = [] } = {}) {
  const words = String(query).toLowerCase().split(/\s+/).filter(Boolean);
  const wanted = new Set(types);
  return items.filter((item) => {
    if (wanted.size > 0 && !wanted.has(item.type)) return false;
    if (words.length === 0) return true;
    const hay = `${item.name ?? ""} ${item.label ?? ""} ${item.url ?? ""}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

/** `{type: count}` for the types that have at least one item, in TYPE_ORDER. */
export function countByType(items) {
  const counts = {};
  for (const type of TYPE_ORDER) {
    const n = items.filter((item) => item.type === type).length;
    if (n > 0) counts[type] = n;
  }
  return counts;
}

/** Sum of known sizes and whether some sizes are unknown. */
export function totalSize(items) {
  let bytes = 0;
  let unknown = 0;
  for (const item of items) {
    if (Number.isFinite(item.size) && item.size >= 0) bytes += item.size;
    else unknown += 1;
  }
  return { bytes, unknown };
}

// ---- Protocol items ------------------------------------------------------------

/** A protocol item `{url, filename?, referrer?, engine?}` with empty fields omitted. */
export function buildItem(url, { filename, referrer, engine } = {}) {
  const item = { url };
  const name = basename(filename);
  if (name) item.filename = name.slice(0, 255);
  const ref = safeReferrer(referrer);
  if (ref) item.referrer = ref.slice(0, 4096);
  if (engine === "http" || engine === "video" || engine === "torrent") item.engine = engine;
  return item;
}

// ---- YouTube -------------------------------------------------------------------

/**
 * Canonical URL for a YouTube watch or Shorts page (other query parameters
 * such as list= and t= removed), or null when the page is not a single video.
 */
export function youtubeVideoUrl(href) {
  let url;
  try {
    url = new URL(href);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  if (!["www.youtube.com", "m.youtube.com", "youtube.com"].includes(url.hostname)) return null;
  let id = "";
  if (url.pathname === "/watch") id = url.searchParams.get("v") || "";
  else {
    const shorts = url.pathname.match(/^\/shorts\/([^/]+)\/?$/);
    if (shorts) id = shorts[1];
  }
  if (!/^[A-Za-z0-9_-]{6,20}$/.test(id)) return null;
  return `https://www.youtube.com/watch?v=${id}`;
}

// ---- Formatting ----------------------------------------------------------------

/** Integer percentage, or null when the total size is unknown. */
export function percent(downloaded, total) {
  if (!Number.isFinite(total) || total <= 0 || !Number.isFinite(downloaded)) return null;
  return Math.max(0, Math.min(100, Math.floor((downloaded / total) * 100)));
}

/** Human-readable byte size, e.g. 1536 -> "1.5 KB". */
export function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(1)} ${units[unit]}`;
}

/** Toolbar badge text for a detected-link count ("" hides the badge). */
export function badgeText(count) {
  const n = Math.floor(Number(count));
  if (!Number.isFinite(n) || n <= 0) return "";
  return n > 999 ? "999+" : String(n);
}

// ---- Browser detection -----------------------------------------------------------

/**
 * Which browser this is, as the protocol's `client.browser` value.
 * `isFirefox` comes from runtime.getBrowserInfo, `isBrave` from navigator.brave,
 * `brands` from navigator.userAgentData.brands (names only).
 */
export function detectBrowser({ userAgent = "", isFirefox = false, isBrave = false, brands = [] } = {}) {
  const names = (Array.isArray(brands) ? brands : []).map((b) => String(b?.brand ?? b).toLowerCase());
  if (isFirefox || /\bFirefox\//.test(userAgent)) return "firefox";
  if (/\bEdg(e|A|iOS)?\//.test(userAgent) || names.includes("microsoft edge")) return "edge";
  if (/\bOPR\//.test(userAgent) || names.includes("opera")) return "opera";
  if (isBrave || names.includes("brave")) return "brave";
  if (/\bVivaldi\//.test(userAgent) || names.includes("vivaldi")) return "vivaldi";
  return "chrome";
}

// ---- Settings --------------------------------------------------------------------

/**
 * Version of the stored options. 1 (or missing) is 2.0.x, where capturing
 * browser downloads was opt-in and the minimum size was in MB (`minSizeMb`).
 */
export const SETTINGS_VERSION = 2;
/** Most entries kept in the per-site exclusion and file-type lists. */
export const MAX_LIST_ENTRIES = 500;

/** Stored options (storage.local). Browser downloads go to pixidl by default. */
export const DEFAULT_SETTINGS = Object.freeze({
  selectionButton: true,
  detectLinks: true,
  showBadge: true,
  youtubeButton: true,
  captureDownloads: true,
  /** Known sizes below this stay in the browser (0 = every size). */
  minSizeKb: 100,
  /** In-page notice (or system notification) when a download goes to pixidl. */
  captureNotice: true,
  /** Host names (and their subdomains) whose downloads are never captured. */
  captureExcludedSites: Object.freeze([]),
  /** File extensions (lower case, no dot) that stay in the browser. */
  captureSkipExtensions: Object.freeze([]),
});

/** Every storage.local key the settings are read from (including legacy ones). */
export const SETTINGS_STORAGE_KEYS = Object.freeze([...Object.keys(DEFAULT_SETTINGS), "settingsVersion", "minSizeMb"]);

/** `www.example.com` -> `example.com`; null when it is not a plain host name. */
export function normalizeHost(input) {
  let host = String(input ?? "").trim().toLowerCase();
  if (!host) return null;
  if (/^[a-z][a-z0-9+.-]*:\/\//.test(host)) host = hostOf(host);
  host = host.replace(/\.$/, "").replace(/^www\./, "");
  if (!host || host.length > 253 || !/^[a-z0-9.-]+$|^\[[0-9a-f:.]+\]$/.test(host) || host.startsWith(".")) return null;
  return host;
}

/** Whether `host` is one of `sites` or a subdomain of one. */
export function hostMatches(host, sites) {
  const h = String(host ?? "").toLowerCase().replace(/\.$/, "");
  if (!h || !Array.isArray(sites)) return false;
  return sites.some((site) => typeof site === "string" && site && (h === site || h.endsWith(`.${site}`) || h === `www.${site}`));
}

/** "zip, .ISO  exe" (or an array) -> ["zip", "iso", "exe"]. */
export function parseExtensionList(input) {
  const parts = Array.isArray(input) ? input : String(input ?? "").split(/[\s,;]+/);
  const out = [];
  for (const part of parts) {
    const ext = String(part ?? "").trim().toLowerCase().replace(/^\*?\./, "");
    if (/^[a-z0-9]{1,10}$/.test(ext) && !out.includes(ext)) out.push(ext);
    if (out.length >= MAX_LIST_ENTRIES) break;
  }
  return out;
}

function sanitizeSites(list) {
  const out = [];
  for (const entry of Array.isArray(list) ? list : []) {
    const host = typeof entry === "string" ? normalizeHost(entry) : null;
    if (host && !out.includes(host)) out.push(host);
    if (out.length >= MAX_LIST_ENTRIES) break;
  }
  return out;
}

/**
 * Coerces stored options into valid values, upgrading 2.0.x options:
 * - capture was opt-in in 2.0 and is on by default since 2.1, so it is turned
 *   on once (2.0 always stored the whole object, so "never changed" cannot be
 *   told apart from "turned off"); after that the stored choice is kept;
 * - `minSizeMb` becomes `minSizeKb` (a 2.0 value of 0 was the default and
 *   takes the new default).
 */
export function sanitizeSettings(stored) {
  const s = stored && typeof stored === "object" ? stored : {};
  const version = Number.isInteger(s.settingsVersion) ? s.settingsVersion : 1;
  const bool = (key) => (typeof s[key] === "boolean" ? s[key] : DEFAULT_SETTINGS[key]);
  let minSizeKb = DEFAULT_SETTINGS.minSizeKb;
  if (version >= SETTINGS_VERSION) {
    const kb = Math.floor(Number(s.minSizeKb));
    if (s.minSizeKb !== undefined && s.minSizeKb !== null && s.minSizeKb !== "" && Number.isFinite(kb)) minSizeKb = kb;
  } else {
    const mb = Math.floor(Number(s.minSizeMb));
    if (Number.isFinite(mb) && mb > 0) minSizeKb = mb * 1024;
  }
  return {
    selectionButton: bool("selectionButton"),
    detectLinks: bool("detectLinks"),
    showBadge: bool("showBadge"),
    youtubeButton: bool("youtubeButton"),
    captureDownloads: version >= SETTINGS_VERSION ? bool("captureDownloads") : true,
    minSizeKb: Math.min(Math.max(minSizeKb, 0), 1_000_000_000),
    captureNotice: bool("captureNotice"),
    captureExcludedSites: sanitizeSites(s.captureExcludedSites),
    captureSkipExtensions: parseExtensionList(s.captureSkipExtensions),
  };
}

/**
 * What to write back to storage.local so the stored options are current:
 * `{set, remove}`, or null when nothing needs to change.
 */
export function migrateSettings(stored) {
  const s = stored && typeof stored === "object" ? stored : {};
  const version = Number.isInteger(s.settingsVersion) ? s.settingsVersion : 1;
  if (version >= SETTINGS_VERSION && !("minSizeMb" in s)) return null;
  return { set: { ...sanitizeSettings(s), settingsVersion: SETTINGS_VERSION }, remove: ["minSizeMb"] };
}

/** Whether a browser download of `sizeBytes` is large enough to hand to pixidl. */
export function meetsMinSize(sizeBytes, minSizeKb) {
  const min = Number(minSizeKb);
  if (!Number.isFinite(min) || min <= 0) return true;
  if (!Number.isFinite(sizeBytes) || sizeBytes <= 0) return true; // unknown size: capture
  return sizeBytes >= min * 1024;
}

// ---- Capturing browser downloads ---------------------------------------------------

/** A failed ping younger than this means pixidl is not reachable: downloads stay in the browser. */
export const UNREACHABLE_GRACE_MS = 2 * 60 * 1000;
/** How long an Alt+click on a link keeps its download in the browser. */
export const BYPASS_TTL_MS = 10 * 1000;

/**
 * Whether pixidl is worth asking, from the cached ping. A recent failure
 * means no; success, no ping yet, or an old failure (the app may have been
 * started since) means yes; the request itself then decides.
 */
export function appReachable(ping, now = Date.now()) {
  if (!ping || typeof ping !== "object") return true;
  if (ping.connected) return ping.integration_enabled !== false;
  const age = now - Number(ping.checked_at);
  return !(Number.isFinite(age) && age >= 0 && age < UNREACHABLE_GRACE_MS);
}

/** Schemes that never leave the browser (and are not downloadable by pixidl anyway). */
const BROWSER_ONLY_SCHEMES = /^(blob|data|file|filesystem|chrome|chrome-extension|moz-extension|edge|about|javascript):/i;

/**
 * Decides whether a browser download is handed to pixidl.
 *
 * `item` is a downloads.DownloadItem (url, finalUrl, referrer, filename,
 * mime, totalBytes, fileSize, incognito, state, byExtensionId). `context`:
 * `{settings, ping, now, bypass: [{url, until}], ownUrls: [url]}` where
 * `bypass` holds Alt+clicked links and `ownUrls` downloads this extension
 * re-started itself.
 *
 * Returns `{capture, reason, url}`; `url` is what to send when capturing.
 */
export function captureDecision(item, { settings = DEFAULT_SETTINGS, ping = null, now = Date.now(), bypass = [], ownUrls = [] } = {}) {
  const no = (reason) => ({ capture: false, reason, url: null });
  if (!item || typeof item !== "object") return no("invalid");
  if (!settings.captureDownloads) return no("disabled");
  if (item.incognito) return no("incognito");
  if (item.state && item.state !== "in_progress") return no("not_in_progress");
  if (item.byExtensionId) return no("by_extension");
  const rawUrl = typeof item.url === "string" ? item.url : "";
  const rawFinal = typeof item.finalUrl === "string" ? item.finalUrl : "";
  if (BROWSER_ONLY_SCHEMES.test(rawUrl) || BROWSER_ONLY_SCHEMES.test(rawFinal)) return no("browser_only_scheme");
  const url = isHttpUrl(rawFinal) ? supportedUrl(rawFinal) : isHttpUrl(rawUrl) ? supportedUrl(rawUrl) : null;
  if (!url) return no("unsupported_url");
  const candidates = [rawUrl, rawFinal].map(supportedUrl).filter(Boolean);
  if ((Array.isArray(ownUrls) ? ownUrls : []).map(supportedUrl).some((u) => u && candidates.includes(u))) return no("own_download");
  const bypassed = (Array.isArray(bypass) ? bypass : []).some((b) => b && Number(b.until) > now && candidates.includes(supportedUrl(b.url)));
  if (bypassed) return no("bypass");
  const sites = settings.captureExcludedSites;
  if (hostMatches(hostOf(rawUrl), sites) || hostMatches(hostOf(rawFinal), sites) || hostMatches(hostOf(item.referrer), sites)) return no("excluded_site");
  const skip = settings.captureSkipExtensions ?? [];
  if (skip.length > 0) {
    const ext = fileExtension(basename(item.filename)) || fileExtension(url);
    if (ext && skip.includes(ext)) return no("skipped_type");
  }
  const size = Number(item.totalBytes) > 0 ? Number(item.totalBytes) : Number(item.fileSize);
  if (!meetsMinSize(size, settings.minSizeKb)) return no("too_small");
  if (!appReachable(ping, now)) return no("app_unreachable");
  return { capture: true, reason: "ok", url };
}

/** The cached result of the last ping, as stored in storage.local `lastPing`. */
export function pingSummary(resp, now = Date.now()) {
  if (resp?.success) {
    return {
      connected: true,
      app_version: String(resp.app_version ?? ""),
      accent_color: safeAccent(resp.accent_color),
      language: typeof resp.language === "string" ? resp.language.slice(0, 16) : "",
      integration_enabled: resp.integration_enabled !== false,
      checked_at: now,
    };
  }
  return { connected: false, error_code: String(resp?.error?.code ?? "internal"), checked_at: now };
}

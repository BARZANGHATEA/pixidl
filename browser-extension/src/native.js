// Native Messaging client for the pixidl desktop app, used by the background
// script and the extension pages. Every call is a one-shot
// runtime.sendNativeMessage: the browser launches the native host, which
// forwards the request to the running app and replies.

import {
  HOST_NAME,
  PROBE_CHUNK,
  batchItems,
  buildEnvelope,
  chunk,
  detectBrowser,
  errorResponse,
  normalizeResponse,
  pingSummary,
} from "./lib.js";

/** WebExtension namespace: promise-based in Firefox (`browser`) and Chromium MV3 (`chrome`). */
export const api = globalThis.browser ?? globalThis.chrome;

export const t = (key, subs) => api.i18n.getMessage(key, subs) || key;

let clientPromise = null;

/** `{browser, version}` sent with every message so the app can list this extension. */
export function clientInfo() {
  clientPromise ??= (async () => {
    let isFirefox = false;
    try {
      isFirefox = typeof api.runtime.getBrowserInfo === "function" && /firefox/i.test((await api.runtime.getBrowserInfo()).name);
    } catch {
      isFirefox = typeof api.runtime.getBrowserInfo === "function";
    }
    const nav = globalThis.navigator ?? {};
    const browser = detectBrowser({
      userAgent: String(nav.userAgent ?? ""),
      isFirefox,
      isBrave: Boolean(nav.brave && typeof nav.brave.isBrave === "function"),
      brands: nav.userAgentData?.brands ?? [],
    });
    return { browser, version: api.runtime.getManifest().version };
  })();
  return clientPromise;
}

/**
 * Sends one protocol message and resolves with the parsed response.
 * Never rejects: a missing or crashed host becomes `app_unavailable`.
 */
export async function send(type, payload = {}) {
  const envelope = buildEnvelope(type, payload, undefined, await clientInfo());
  try {
    const resp = await api.runtime.sendNativeMessage(HOST_NAME, envelope);
    return normalizeResponse(resp, envelope.id);
  } catch (err) {
    const message = err?.message || api.runtime.lastError?.message || "Native host not available";
    return errorResponse("app_unavailable", message, envelope.id);
  }
}

/** Pings the app and caches the result (connection, version, accent, language). */
export async function pingAndCache() {
  const resp = await send("ping");
  const summary = pingSummary(resp);
  try {
    const { lastPing } = await api.storage.local.get("lastPing");
    // Keep the last known accent color while the app is not running.
    const merged = resp.success ? summary : { ...summary, accent_color: lastPing?.accent_color };
    await api.storage.local.set({ lastPing: merged });
  } catch {
    // Storage is best effort.
  }
  return resp;
}

/** The cached ping result, or null. */
export async function cachedPing() {
  try {
    return (await api.storage.local.get("lastPing")).lastPing ?? null;
  } catch {
    return null;
  }
}

/** Whether nothing more can get through (app gone or integration off). */
export const isFatal = (error) => error?.code === "app_unavailable" || error?.code === "unauthorized" || error?.code === "unsupported_version";

/**
 * Sends many items as add_multiple_downloads, split into protocol-sized
 * batches. Resolves with `{added, total, error}` where `error` is the first
 * failure (batch-level or per item), if any.
 */
export async function sendMany(items) {
  let added = 0;
  let error = null;
  for (const batch of batchItems(items)) {
    const resp = await send("add_multiple_downloads", { items: batch });
    if (resp.success) {
      added += Number(resp.added) || 0;
      if (!error && Array.isArray(resp.results)) {
        const failed = resp.results.find((r) => r && r.success === false);
        if (failed) error = failed.error && typeof failed.error === "object" ? failed.error : { code: "internal", message: String(failed.error ?? "") };
      }
    } else {
      error ??= resp.error;
      if (isFatal(resp.error)) break;
    }
  }
  return { added, total: items.length, error };
}

/**
 * Probes links in chunks of PROBE_CHUNK, `parallel` requests at a time.
 * `onChunk(startIndex, results, error)` is called as each chunk finishes;
 * probing stops early on a fatal error. Resolves with the first error, if any.
 */
export async function probeAll(items, onChunk, { parallel = 2 } = {}) {
  const chunks = chunk(items, PROBE_CHUNK).map((list, i) => ({ list, start: i * PROBE_CHUNK }));
  let firstError = null;
  let stop = false;
  const worker = async () => {
    while (!stop && chunks.length > 0) {
      const { list, start } = chunks.shift();
      const resp = await send("probe_links", { items: list });
      if (resp.success && Array.isArray(resp.results)) {
        onChunk(start, resp.results, null);
      } else {
        firstError ??= resp.error;
        onChunk(start, [], resp.error);
        if (isFatal(resp.error)) stop = true;
      }
    }
  };
  await Promise.all(Array.from({ length: Math.max(1, parallel) }, worker));
  return firstError;
}

/** Localized, user-facing text for a protocol error. */
export function errorText(error) {
  switch (error?.code) {
    case "app_unavailable":
      return t("errAppUnavailable");
    case "unauthorized":
      return t("errUnauthorized");
    case "invalid_url":
      return t("errInvalidUrl");
    case "unsupported_version":
      return t("errUnsupportedVersion");
    default:
      return error?.message || t("errUnknown");
  }
}

/** Applies the extension's language direction and translations to a page. */
export function localizePage(doc = document) {
  // "@@bidi_dir" is the standard source; "uiDirection" (set per locale) backs it
  // up because some renderers report "ltr" for an RTL message catalog.
  const rtl = api.i18n.getMessage("@@bidi_dir") === "rtl" || api.i18n.getMessage("uiDirection") === "rtl";
  doc.documentElement.dir = rtl ? "rtl" : "ltr";
  doc.documentElement.lang = api.i18n.getUILanguage();
  for (const el of doc.querySelectorAll("[data-i18n]")) el.textContent = t(el.dataset.i18n);
  for (const el of doc.querySelectorAll("[data-i18n-placeholder]")) el.placeholder = t(el.dataset.i18nPlaceholder);
  for (const el of doc.querySelectorAll("[data-i18n-title]")) el.title = t(el.dataset.i18nTitle);
  for (const el of doc.querySelectorAll("[data-i18n-aria]")) el.setAttribute("aria-label", t(el.dataset.i18nAria));
}

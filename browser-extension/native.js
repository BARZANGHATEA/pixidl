// Native Messaging client for the pixidl desktop app.
// Every call is a one-shot runtime.sendNativeMessage: the browser launches the
// native host, which forwards the request to the running app and replies.

import { HOST_NAME, batchItems, buildEnvelope, errorResponse, normalizeResponse } from "./lib.js";

/** WebExtension namespace: promise-based in Firefox (`browser`) and Chrome MV3 (`chrome`). */
export const api = globalThis.browser ?? globalThis.chrome;

/**
 * Sends one protocol message and resolves with the parsed response.
 * Never rejects: a missing host or a crashed host becomes `app_unavailable`.
 */
export async function send(type, payload = {}) {
  const envelope = buildEnvelope(type, payload);
  try {
    const resp = await api.runtime.sendNativeMessage(HOST_NAME, envelope);
    return normalizeResponse(resp, envelope.id);
  } catch (err) {
    const message = err?.message || api.runtime.lastError?.message || "Native host not available";
    return errorResponse("app_unavailable", message, envelope.id);
  }
}

/**
 * Sends many items as add_multiple_downloads, split into protocol-sized
 * batches. Resolves with `{added, total, error}` where `error` is the first
 * batch-level failure (if any).
 */
export async function sendMany(items) {
  let added = 0;
  let error = null;
  for (const batch of batchItems(items)) {
    const resp = await send("add_multiple_downloads", { items: batch });
    if (resp.success) {
      added += Number(resp.added) || 0;
    } else {
      error ??= resp.error;
      // Nothing else will get through if the app is gone or integration is off.
      if (resp.error.code === "app_unavailable" || resp.error.code === "unauthorized") break;
    }
  }
  return { added, total: items.length, error };
}

/** Localized, user-facing text for a protocol error. */
export function errorText(error) {
  const t = (key, sub) => api.i18n.getMessage(key, sub) || "";
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

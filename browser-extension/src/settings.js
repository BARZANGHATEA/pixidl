// Extension options stored in storage.local (local to this browser profile).

import { api, cachedPing } from "./native.js";
import { DEFAULT_SETTINGS, safeAccent, sanitizeSettings } from "./lib.js";

const KEYS = Object.keys(DEFAULT_SETTINGS);

export async function loadSettings() {
  try {
    return sanitizeSettings(await api.storage.local.get(KEYS));
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export async function saveSettings(patch) {
  const next = sanitizeSettings({ ...(await loadSettings()), ...patch });
  await api.storage.local.set(next);
  return next;
}

/** Uses the app's accent color (from the last ping) for `--accent` on a page. */
export async function applyAccent(doc = document) {
  const ping = await cachedPing();
  const accent = safeAccent(ping?.accent_color);
  doc.documentElement.style.setProperty("--accent", accent);
  return accent;
}

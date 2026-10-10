// Extension options stored in storage.local (local to this browser profile).

import { api, cachedPing } from "./native.js";
import { SETTINGS_STORAGE_KEYS, SETTINGS_VERSION, migrateSettings, safeAccent, sanitizeSettings } from "./lib.js";

export async function loadSettings() {
  try {
    return sanitizeSettings(await api.storage.local.get([...SETTINGS_STORAGE_KEYS]));
  } catch {
    return sanitizeSettings({ settingsVersion: SETTINGS_VERSION });
  }
}

export async function saveSettings(patch) {
  const next = sanitizeSettings({ ...(await loadSettings()), ...patch, settingsVersion: SETTINGS_VERSION });
  await api.storage.local.set({ ...next, settingsVersion: SETTINGS_VERSION });
  try {
    await api.storage.local.remove("minSizeMb");
  } catch {
    // Legacy key; harmless if it stays.
  }
  return next;
}

/** Brings options stored by an older version up to date (install, update, startup). */
export async function migrateStoredSettings() {
  try {
    const plan = migrateSettings(await api.storage.local.get([...SETTINGS_STORAGE_KEYS]));
    if (!plan) return false;
    await api.storage.local.set(plan.set);
    if (plan.remove.length) await api.storage.local.remove(plan.remove);
    return true;
  } catch {
    return false;
  }
}

/** Uses the app's accent color (from the last ping) for `--accent` on a page. */
export async function applyAccent(doc = document) {
  const ping = await cachedPing();
  const accent = safeAccent(ping?.accent_color);
  doc.documentElement.style.setProperty("--accent", accent);
  return accent;
}

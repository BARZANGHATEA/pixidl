// Extension options stored in storage.local (local to this browser profile).

import { api } from "./native.js";
import { DEFAULT_SETTINGS, sanitizeSettings } from "./lib.js";

export async function loadSettings() {
  try {
    return sanitizeSettings(await api.storage.local.get({ ...DEFAULT_SETTINGS }));
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export async function saveSettings(patch) {
  const next = sanitizeSettings({ ...(await loadSettings()), ...patch });
  await api.storage.local.set(next);
  return next;
}

import { useEffect } from "react";
import type { Settings } from "../types";

/** Applies theme (light/dark/system) and accent color from settings. */
export function useTheme(settings: Settings | null) {
  useEffect(() => {
    if (!settings) return;
    const root = document.documentElement;
    root.style.setProperty("--accent", settings.accentColor);
    const media = window.matchMedia?.("(prefers-color-scheme: dark)");
    const apply = () => {
      const dark = settings.theme === "dark" || (settings.theme === "system" && !!media?.matches);
      root.dataset.theme = dark ? "dark" : "light";
    };
    apply();
    media?.addEventListener?.("change", apply);
    return () => media?.removeEventListener?.("change", apply);
  }, [settings?.theme, settings?.accentColor]);
}

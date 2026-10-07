// Translation system. User-facing strings live in en.json / fa.json only.
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./en.json";
import fa from "./fa.json";

export const LANGUAGES = [
  { code: "en", label: "English", dir: "ltr" as const },
  { code: "fa", label: "فارسی", dir: "rtl" as const },
];

export function dirFor(lang: string): "ltr" | "rtl" {
  return LANGUAGES.find((l) => l.code === lang)?.dir ?? "ltr";
}

/** Applies language + direction to the document (RTL for Persian). */
export function applyLanguage(lang: string) {
  const code = LANGUAGES.some((l) => l.code === lang) ? lang : "en";
  void i18n.changeLanguage(code);
  document.documentElement.lang = code;
  document.documentElement.dir = dirFor(code);
}

void i18n.use(initReactI18next).init({
  resources: { en: { translation: en }, fa: { translation: fa } },
  lng: "en",
  fallbackLng: "en",
  interpolation: { escapeValue: false },
  returnNull: false,
});

export default i18n;

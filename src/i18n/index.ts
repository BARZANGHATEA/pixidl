// Translation system. User-facing strings live in the <code>.json files next to
// this module; en.json is the source of truth and every other file mirrors it.
import i18n from "i18next";
import { initReactI18next } from "react-i18next";

export type Language = { code: string; label: string; english: string; dir: "ltr" | "rtl" };

/** Shown in this order in the language pickers (native names). */
export const LANGUAGES: Language[] = [
  { code: "en", label: "English", english: "English", dir: "ltr" },
  { code: "fa", label: "فارسی", english: "Persian", dir: "rtl" },
  { code: "ckb", label: "کوردی (سۆرانی)", english: "Kurdish (Sorani)", dir: "rtl" },
  { code: "kmr", label: "Kurdî (Kurmancî)", english: "Kurdish (Kurmanji)", dir: "ltr" },
  { code: "ar", label: "العربية", english: "Arabic", dir: "rtl" },
  { code: "tr", label: "Türkçe", english: "Turkish", dir: "ltr" },
  { code: "de", label: "Deutsch", english: "German", dir: "ltr" },
  { code: "fr", label: "Français", english: "French", dir: "ltr" },
  { code: "es", label: "Español", english: "Spanish", dir: "ltr" },
  { code: "pt", label: "Português", english: "Portuguese", dir: "ltr" },
  { code: "ru", label: "Русский", english: "Russian", dir: "ltr" },
  { code: "zh", label: "简体中文", english: "Chinese (Simplified)", dir: "ltr" },
];

const files = import.meta.glob<Record<string, unknown>>("./*.json", { eager: true, import: "default" });
const resources = Object.fromEntries(
  Object.entries(files).map(([path, translation]) => [path.replace(/^\.\/|\.json$/g, ""), { translation }]),
);

/** Languages that have a translation file (all of LANGUAGES in a release build). */
export const AVAILABLE_LANGUAGES = LANGUAGES.filter((l) => l.code in resources);

export function dirFor(lang: string): "ltr" | "rtl" {
  return LANGUAGES.find((l) => l.code === lang)?.dir ?? "ltr";
}

/** BCP 47 tag for Intl (number/date formatting). Kurmanji is "ku" in CLDR. */
export function intlLocale(lang: string): string {
  return lang === "kmr" ? "ku" : lang;
}

/** Applies language + direction to the document (RTL for Persian, Arabic, Sorani). */
export function applyLanguage(lang: string) {
  const code = AVAILABLE_LANGUAGES.some((l) => l.code === lang) ? lang : "en";
  void i18n.changeLanguage(code);
  document.documentElement.lang = code;
  document.documentElement.dir = dirFor(code);
}

void i18n.use(initReactI18next).init({
  resources,
  lng: "en",
  fallbackLng: "en",
  interpolation: { escapeValue: false },
  returnNull: false,
});

export default i18n;

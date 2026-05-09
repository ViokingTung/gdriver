import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import LanguageDetector from "i18next-browser-languagedetector";

import en from "./locales/en.json";

const SUPPORTED = [
  "en",
  "zh-CN",
  "zh-TW",
  "ja",
  "ko",
  "de",
  "fr",
  "es",
  "pt-BR",
  "ru",
  "it",
  "ar",
];

/**
 * Load a language pack on demand via Vite dynamic import.
 * Returns the parsed JSON module (default export).
 */
async function loadLocale(lang: string): Promise<Record<string, unknown>> {
  switch (lang) {
    case "zh-CN":
      return (await import("./locales/zh-CN.json")).default;
    case "zh-TW":
      return (await import("./locales/zh-TW.json")).default;
    case "ja":
      return (await import("./locales/ja.json")).default;
    case "ko":
      return (await import("./locales/ko.json")).default;
    case "de":
      return (await import("./locales/de.json")).default;
    case "fr":
      return (await import("./locales/fr.json")).default;
    case "es":
      return (await import("./locales/es.json")).default;
    case "pt-BR":
      return (await import("./locales/pt-BR.json")).default;
    case "ru":
      return (await import("./locales/ru.json")).default;
    case "it":
      return (await import("./locales/it.json")).default;
    case "ar":
      return (await import("./locales/ar.json")).default;
    default:
      return en;
  }
}

/**
 * Custom backend that lazy-loads language bundles via dynamic import().
 * Only `en` is bundled eagerly; all others are code-split by Vite.
 */
const lazyBackend = {
  type: "backend" as const,
  read(
    language: string,
    _namespace: string,
    callback: (err: Error | null, data?: Record<string, unknown>) => void,
  ) {
    loadLocale(language)
      .then((data) => callback(null, data))
      .catch((err) => callback(err));
  },
};

const RTL_LANGUAGES = new Set(["ar"]);

function applyDirection(lang: string) {
  const base = lang.split("-")[0] ?? lang;
  const dir = RTL_LANGUAGES.has(base) ? "rtl" : "ltr";
  document.documentElement.setAttribute("dir", dir);
  document.documentElement.setAttribute("lang", lang);
}

i18n
  .use(lazyBackend)
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      en: { translation: en },
    },
    supportedLngs: SUPPORTED,
    fallbackLng: "en",
    detection: {
      order: ["localStorage", "navigator"],
      caches: ["localStorage"],
    },
    interpolation: {
      escapeValue: false,
    },
  });

// Apply direction on initial load and whenever language changes.
applyDirection(i18n.language ?? "en");
i18n.on("languageChanged", applyDirection);

export default i18n;

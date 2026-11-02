// Interface translations. Each file in locales/ adds one language to PF_LOCALES; missing keys fall back to English.
const PF_LOCALES = {};

/** Languages the interface is translated into, by code. */
const PF_UI_LANGUAGES = ["en", "es", "pt", "fr", "de", "it", "nl", "pl", "tr", "ru", "uk", "ja", "ko", "zh", "id", "vi"];

let pfLang = "en";

function setLanguage(code) {
  pfLang = PF_LOCALES[code] ? code : "en";
  document.documentElement.lang = pfLang;
}

/** Translated text for `key`, with {placeholders} filled from `vars`. */
function t(key, vars = {}) {
  const text = (PF_LOCALES[pfLang] && PF_LOCALES[pfLang][key]) ?? PF_LOCALES.en[key] ?? key;
  return text.replace(/\{(\w+)\}/g, (m, name) => (name in vars ? vars[name] : m));
}

/** "pt" → "Português" (in its own language) or "Portuguese" (in the interface language). */
function languageName(code, inOwnLanguage = false) {
  try {
    const name = new Intl.DisplayNames([inOwnLanguage ? code : pfLang], { type: "language" }).of(code);
    return name.charAt(0).toLocaleUpperCase(code) + name.slice(1);
  } catch {
    return code;
  }
}

/** "English and Turkish", in the interface language. */
function languageList(codes) {
  const names = codes.map((c) => languageName(c));
  try {
    return new Intl.ListFormat(pfLang, { type: "conjunction" }).format(names);
  } catch {
    return names.join(", ");
  }
}

/** Fills every element that has data-i18n="key" (text) or data-i18n-title="key" (tooltip). */
function translatePage() {
  for (const el of document.querySelectorAll("[data-i18n]")) el.textContent = t(el.dataset.i18n);
  for (const el of document.querySelectorAll("[data-i18n-title]")) el.title = t(el.dataset.i18nTitle);
}

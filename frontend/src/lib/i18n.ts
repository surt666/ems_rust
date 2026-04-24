import da from '../../../translations/da.json';
import en from '../../../translations/en.json';

const locales: Record<string, Record<string, any>> = { da, en };

export type Locale = 'da' | 'en';
export const defaultLocale: Locale = 'da';

export function getLocale(): Locale {
  if (typeof localStorage !== 'undefined') {
    const stored = localStorage.getItem('lang');
    if (stored === 'da' || stored === 'en') return stored;
  }
  return defaultLocale;
}

export function setLocale(lang: Locale): void {
  localStorage.setItem('lang', lang);
}

export function t(key: string, lang?: Locale, params?: Record<string, string | number>): string {
  const locale = lang ?? getLocale();
  const keys = key.split('.');
  let value: any = locales[locale] ?? locales[defaultLocale];
  for (const k of keys) {
    value = value?.[k];
  }
  if (typeof value !== 'string') return key;
  if (params) {
    return value.replace(/\{(\w+)\}/g, (_, k: string) => String(params[k] ?? `{${k}}`));
  }
  return value;
}

export function applyTranslations(): void {
  const lang = getLocale();
  document.documentElement.lang = lang;

  document.querySelectorAll('[data-i18n]').forEach(el => {
    const key = el.getAttribute('data-i18n');
    if (key) el.textContent = t(key, lang);
  });

  document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
    const key = el.getAttribute('data-i18n-placeholder');
    if (key) (el as HTMLInputElement).placeholder = t(key, lang);
  });

  document.querySelectorAll('[data-i18n-aria]').forEach(el => {
    const key = el.getAttribute('data-i18n-aria');
    if (key) el.setAttribute('aria-label', t(key, lang));
  });

  document.querySelectorAll('[data-i18n-title]').forEach(el => {
    const key = el.getAttribute('data-i18n-title');
    if (key) document.title = t(key, lang);
  });
}

// The asset handler prepends ENGLISH from locales/en.json to this module.
const SUPPORTED = Object.freeze({ en: 'en', zh: 'zh-CN', ja: 'ja', ko: 'ko', ru: 'ru' });
const ATTRIBUTES = ['aria-label', 'placeholder', 'title', 'alt', 'content'];
const bindings = new WeakMap();
let messages = ENGLISH;
let locale = 'en';
let selection = 0;

function canonicalLanguage(value) {
  if (typeof value !== 'string') return '';
  try { return Intl.getCanonicalLocales(value.trim().replaceAll('_', '-'))[0] || ''; }
  catch { return ''; }
}

export function resolveLocale(preferences) {
  for (const preference of preferences) {
    const base = canonicalLanguage(preference).split('-')[0];
    if (Object.hasOwn(SUPPORTED, base)) return SUPPORTED[base];
  }
  return 'en';
}

function browserPreferences() {
  const preferences = Array.from(navigator.languages || []);
  preferences.push(navigator.language);
  return preferences;
}

export function hasMessage(key) {
  return Object.hasOwn(ENGLISH, key);
}

function formatValue(value) {
  if (typeof value === 'number') return new Intl.NumberFormat(locale).format(value);
  if (value && typeof value === 'object') return formatStructuredValue(value);
  return String(value ?? '');
}

function formatStructuredValue(value) {
  if (Object.hasOwn(value, 'date')) {
    return new Intl.DateTimeFormat(locale, {
      year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit',
    }).format(new Date(value.date));
  }
  return new Intl.NumberFormat(locale, value.options).format(value.number);
}

export function t(key, values = {}) {
  const suffix = typeof values.count === 'number' ? `.${new Intl.PluralRules(locale).select(values.count)}` : '';
  const template = messages[key + suffix] ?? ENGLISH[key + suffix] ?? '';
  return template.replace(/\{([A-Za-z][A-Za-z0-9_]*)\}/g, (_, name) => {
    return Object.hasOwn(values, name) ? formatValue(values[name]) : '';
  });
}

function remember(node, target, key, values) {
  const saved = bindings.get(node) || {};
  saved[target] = values;
  bindings.set(node, saved);
  node.setAttribute(target, key);
}

export function text(node, key, values = {}) {
  remember(node, 'data-i18n', key, values);
  node.textContent = t(key, values);
  return node;
}

export function attribute(node, name, key, values = {}) {
  if (!ATTRIBUTES.includes(name)) return node;
  remember(node, `data-i18n-${name}`, key, values);
  node.setAttribute(name, t(key, values));
  return node;
}

export function clearText(node) {
  node.removeAttribute('data-i18n');
  bindings.delete(node);
  node.textContent = '';
}

function applyBinding(node, target, write) {
  const key = node.getAttribute(target);
  const values = bindings.get(node)?.[target];
  write(t(key, values));
}

function localize() {
  document.querySelectorAll('[data-i18n]').forEach((node) => {
    applyBinding(node, 'data-i18n', (value) => { node.textContent = value; });
  });
  for (const name of ATTRIBUTES) {
    const target = `data-i18n-${name}`;
    document.querySelectorAll(`[${target}]`).forEach((node) => {
      applyBinding(node, target, (value) => node.setAttribute(name, value));
    });
  }
}

function validCatalog(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const entries = Object.entries(value);
  return entries.length > 0 && entries.every(([key, item]) => {
    return hasMessage(key) && typeof item === 'string' && item.trim().length > 0;
  });
}

async function loadCatalog(chosen) {
  if (chosen === 'en') return { locale: 'en', messages: ENGLISH };
  try {
    const response = await fetch(`/assets/locales/${chosen}.json`, {
      credentials: 'same-origin', redirect: 'error', cache: 'no-cache', signal: AbortSignal.timeout(3000),
    });
    if (!response.ok) throw new Error();
    const catalog = await response.json();
    if (!validCatalog(catalog)) throw new Error();
    return { locale: chosen, messages: { ...ENGLISH, ...catalog } };
  } catch { return { locale: 'en', messages: ENGLISH }; }
}

async function selectLanguage() {
  const current = ++selection;
  const catalog = await loadCatalog(resolveLocale(browserPreferences()));
  if (current !== selection) return;
  locale = catalog.locale;
  messages = catalog.messages;
  document.documentElement.lang = locale;
  localize();
  window.dispatchEvent(new Event('memehub:localechange'));
}

// English text and event handlers remain available during the locale request.
localize();
export const ready = selectLanguage();
window.addEventListener('languagechange', selectLanguage);

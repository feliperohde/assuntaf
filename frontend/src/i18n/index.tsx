'use client';

/**
 * App (UI) language. Separate from the transcription language, which only
 * tells the speech model what language is spoken in the meeting.
 *
 * Messages live in ./messages; English is the fallback for any missing key.
 * The preference is per device (localStorage); "system" follows the OS locale.
 */

import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import { en, MessageKey } from './messages/en';
import { ptBR } from './messages/pt-BR';

export type Locale = 'en' | 'pt-BR';
export type LanguagePreference = 'system' | Locale;
export type { MessageKey };

export const LOCALES: { value: Locale; label: string }[] = [
  { value: 'en', label: 'English' },
  { value: 'pt-BR', label: 'Português (Brasil)' },
];

const MESSAGES: Record<Locale, Record<MessageKey, string>> = { en, 'pt-BR': ptBR };
const STORAGE_KEY = 'assunta.uiLanguage';

export type TranslateVars = Record<string, string | number>;
export type Translate = (key: MessageKey, vars?: TranslateVars) => string;

function systemLocale(): Locale {
  if (typeof navigator === 'undefined') return 'en';
  const languages = navigator.languages?.length ? navigator.languages : [navigator.language];
  return languages.some(l => l?.toLowerCase().startsWith('pt')) ? 'pt-BR' : 'en';
}

export function resolveLocale(preference: LanguagePreference): Locale {
  return preference === 'system' ? systemLocale() : preference;
}

export function translate(locale: Locale, key: MessageKey, vars?: TranslateVars): string {
  const template = MESSAGES[locale][key] ?? en[key] ?? key;
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (match, name) => (name in vars ? String(vars[name]) : match));
}

interface I18nContextValue {
  locale: Locale;
  preference: LanguagePreference;
  setPreference: (preference: LanguagePreference) => void;
  t: Translate;
}

const I18nContext = createContext<I18nContextValue | null>(null);

function readPreference(): LanguagePreference {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === 'system' || stored === 'en' || stored === 'pt-BR') return stored;
  } catch {
    // Storage unavailable: follow the system
  }
  return 'system';
}

export function I18nProvider({ children }: { children: React.ReactNode }) {
  const [preference, setPreferenceState] = useState<LanguagePreference>('system');
  const [locale, setLocale] = useState<Locale>('en');

  useEffect(() => {
    const stored = readPreference();
    setPreferenceState(stored);
    setLocale(resolveLocale(stored));
  }, []);

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  const setPreference = useCallback((next: LanguagePreference) => {
    setPreferenceState(next);
    setLocale(resolveLocale(next));
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Applies for this session only
    }
  }, []);

  const t = useCallback<Translate>((key, vars) => translate(locale, key, vars), [locale]);
  const value = useMemo(() => ({ locale, preference, setPreference, t }), [locale, preference, setPreference, t]);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  const context = useContext(I18nContext);
  if (!context) {
    // Outside the provider (tests, isolated renders): English
    return { locale: 'en', preference: 'en', setPreference: () => {}, t: (key, vars) => translate('en', key, vars) };
  }
  return context;
}

/** Date formatting in the UI language. */
export function formatDate(locale: Locale, value: string | Date, options?: Intl.DateTimeFormatOptions): string {
  const date = typeof value === 'string' ? new Date(value) : value;
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleDateString(locale, options);
}

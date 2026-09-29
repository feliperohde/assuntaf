'use client';

import React from 'react';
import { Languages } from 'lucide-react';
import { LOCALES, LanguagePreference, useI18n } from '@/i18n';

/** Picks the app's interface language (not the transcription language). */
export function UiLanguageSelect({ compact = false }: { compact?: boolean }) {
  const { t, preference, setPreference } = useI18n();
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <Languages className="h-4 w-4 text-gray-600" />
        <h4 className="text-sm font-medium text-gray-900">{t('uiLanguage.title')}</h4>
      </div>
      <select
        value={preference}
        onChange={e => setPreference(e.target.value as LanguagePreference)}
        className="w-full px-3 py-2 text-sm bg-white border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-1 focus:ring-blue-500"
        aria-label={t('uiLanguage.title')}
      >
        <option value="system">{t('uiLanguage.system')}</option>
        {LOCALES.map(locale => (
          <option key={locale.value} value={locale.value}>
            {locale.label}
          </option>
        ))}
      </select>
      {!compact && <p className="text-xs text-gray-500">{t('uiLanguage.help')}</p>}
    </div>
  );
}

'use client';

import { createContext, useCallback, useContext, useEffect, useState, ReactNode } from 'react';
import { NextIntlClientProvider } from 'next-intl';
import enMessages from '../../messages/en.json';
import ruMessages from '../../messages/ru.json';

export type AppLocale = 'ru' | 'en';

const LOCALE_STORAGE_KEY = 'meetily_locale';
const VALID_LOCALES: readonly AppLocale[] = ['ru', 'en'];

// Same localStorage-preference pattern as app-theme.ts: default locale
// (ru) unless the user picked otherwise in Settings.
function getSavedAppLocale(): AppLocale {
  if (typeof window === 'undefined') return 'ru';
  const saved = localStorage.getItem(LOCALE_STORAGE_KEY);
  return (VALID_LOCALES as readonly string[]).includes(saved ?? '') ? (saved as AppLocale) : 'ru';
}

const MESSAGES: Record<AppLocale, typeof enMessages> = {
  en: enMessages,
  ru: ruMessages,
};

interface LocaleContextValue {
  locale: AppLocale;
  setLocale: (locale: AppLocale) => void;
}

const LocaleContext = createContext<LocaleContextValue | null>(null);

export function LocaleProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<AppLocale>('ru');

  useEffect(() => {
    setLocaleState(getSavedAppLocale());
  }, []);

  const setLocale = useCallback((next: AppLocale) => {
    setLocaleState(next);
    if (typeof window !== 'undefined') localStorage.setItem(LOCALE_STORAGE_KEY, next);
  }, []);

  return (
    <LocaleContext.Provider value={{ locale, setLocale }}>
      <NextIntlClientProvider locale={locale} messages={MESSAGES[locale]}>
        {children}
      </NextIntlClientProvider>
    </LocaleContext.Provider>
  );
}

export function useAppLocale() {
  const ctx = useContext(LocaleContext);
  if (!ctx) throw new Error('useAppLocale must be used within LocaleProvider');
  return ctx;
}

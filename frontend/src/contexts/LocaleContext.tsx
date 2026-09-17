'use client';

import { createContext, useCallback, useContext, useEffect, useState, ReactNode } from 'react';
import { NextIntlClientProvider } from 'next-intl';
import enMessages from '../../messages/en.json';
import ruMessages from '../../messages/ru.json';

export type AppLocale = 'ru' | 'en';

const LOCALE_STORAGE_KEY = 'meetily_locale';

/**
 * The machine's own time zone. Without one next-intl reports an error on
 * every start (shown as the red badge in development), and dates would be
 * formatted in whatever zone the renderer guessed.
 */
const TIME_ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone;
const VALID_LOCALES: readonly AppLocale[] = ['ru', 'en'];

// First run has no stored preference, so follow the system language: a Russian
// locale gets `ru`, everything else falls back to `en`.
function detectSystemLocale(): AppLocale {
  if (typeof navigator === 'undefined') return 'ru';
  const candidates = [navigator.language, ...(navigator.languages ?? [])];
  return candidates.some((tag) => tag?.toLowerCase().startsWith('ru')) ? 'ru' : 'en';
}

// Same localStorage-preference pattern as app-theme.ts: the user's choice in
// Settings wins, otherwise the system language decides.
function getSavedAppLocale(): AppLocale {
  if (typeof window === 'undefined') return 'ru';
  const saved = localStorage.getItem(LOCALE_STORAGE_KEY);
  return (VALID_LOCALES as readonly string[]).includes(saved ?? '')
    ? (saved as AppLocale)
    : detectSystemLocale();
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
      <NextIntlClientProvider locale={locale} messages={MESSAGES[locale]} timeZone={TIME_ZONE}>
        {children}
      </NextIntlClientProvider>
    </LocaleContext.Provider>
  );
}

/**
 * Read the active catalog outside the provider. `app/layout.tsx` renders
 * LocaleProvider, so it cannot use `useTranslations` for its own toasts; the
 * saved preference in localStorage is the same value the provider starts from.
 */
export function getLocaleMessages(): typeof enMessages {
  return MESSAGES[getSavedAppLocale()];
}

export function useAppLocale() {
  const ctx = useContext(LocaleContext);
  if (!ctx) throw new Error('useAppLocale must be used within LocaleProvider');
  return ctx;
}

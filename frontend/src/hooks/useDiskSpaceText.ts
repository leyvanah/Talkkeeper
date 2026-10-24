import { useCallback } from 'react';
import { useLocale, useTranslations } from 'next-intl';
import { noSpaceText } from '@/lib/disk-space';

/**
 * Turns a download error into what the window shows: a lack of disk space in
 * the interface language with both sizes, or `null` for any other error.
 * Takes the language from next-intl, which every place that shows these
 * errors already sits under (toasts included).
 */
export function useDiskSpaceText(): (error: unknown) => string | null {
  const t = useTranslations('common');
  const locale = useLocale();
  return useCallback(
    (error: unknown) => noSpaceText(error, locale, (values) => t('noDiskSpace', values)),
    [locale, t],
  );
}

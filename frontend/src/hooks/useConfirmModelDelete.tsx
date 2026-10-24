'use client';

import { useState } from 'react';
import { useLocale, useTranslations } from 'next-intl';
import { ConfirmAction, type ConfirmRequest } from '@/components/ui/confirm-action';
import { formatDiskSize } from '@/lib/disk-space';

const MEBIBYTE = 1024 * 1024;

/**
 * Asks before a downloaded model is deleted: getting it back means
 * downloading it again, hundreds of megabytes at a time.
 *
 * `ask(name, sizeMb, run)` opens the question; render `dialog` once in the
 * component. A size of 0 (unknown) is left out of the title.
 */
export function useConfirmModelDelete() {
  const t = useTranslations('settings');
  const tc = useTranslations('common');
  const locale = useLocale();
  const [request, setRequest] = useState<ConfirmRequest | null>(null);

  const ask = (model: string, sizeMb: number, run: () => Promise<void> | void) =>
    setRequest({
      title: sizeMb > 0
        ? t('modelDeleteConfirmTitle', { model, size: formatDiskSize(sizeMb * MEBIBYTE, locale) })
        : t('modelDeleteConfirmTitleNoSize', { model }),
      description: t('modelDeleteConfirmDescription'),
      confirmLabel: tc('delete'),
      busyLabel: tc('deleting'),
      run,
    });

  const dialog = <ConfirmAction request={request} onClose={() => setRequest(null)} />;

  return { ask, dialog };
}

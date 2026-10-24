import { useMemo } from 'react';
import { useTranslations } from 'next-intl';
import type { SpeakerWords } from '@/lib/speaker-label';

/** The interface's words for the capture labels, for `speakerLabel`. */
export function useSpeakerWords(): SpeakerWords {
  const t = useTranslations('meetingDetails');
  return useMemo(
    () => ({
      you: t('speakerLabelYou'),
      youWithName: (name: string) => t('speakerYouWithName', { name }),
      guest: t('speakerLabelGuest'),
      numbered: (number: number) => t('speakerLabelNumbered', { number }),
    }),
    [t],
  );
}

"use client";

/**
 * What is done with a summary once it exists: copy it. Exporting the meeting
 * is in the window's header, with the rest of what is done with the meeting.
 *
 * There is no "Save" here: the page has no editor for the summary or the
 * title, so it only ever saved the same summary again (tasks#89). It comes
 * back with an editor.
 */

import { Copy } from 'lucide-react';
import { useTranslations } from 'next-intl';

interface SummaryUpdaterButtonGroupProps {
  onCopy: () => Promise<void>;
  hasSummary: boolean;
}

export function SummaryUpdaterButtonGroup({
  onCopy,
  hasSummary
}: SummaryUpdaterButtonGroupProps) {
  const t = useTranslations('meetingDetails');

  const quiet =
    'flex h-8 shrink-0 items-center justify-center gap-1.5 rounded-lg px-2 text-sm text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)] disabled:opacity-40 disabled:hover:bg-transparent';

  return (
    <div className="flex items-center gap-0.5">
      <button
        type="button"
        className={`${quiet} w-8 px-0`}
        title={t('copySummary')}
        aria-label={t('copySummary')}
        onClick={() => {
          onCopy();
        }}
        disabled={!hasSummary}
      >
        <Copy size={16} />
      </button>
    </div>
  );
}

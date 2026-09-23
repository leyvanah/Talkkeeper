"use client";

/**
 * What is done with a summary once it exists: save the corrections to it and
 * copy it. Exporting the meeting is in the window's header, with the rest of
 * what is done with the meeting.
 */

import { Copy, Loader2, Save } from 'lucide-react';
import { useTranslations } from 'next-intl';

interface SummaryUpdaterButtonGroupProps {
  isSaving: boolean;
  isDirty: boolean;
  onSave: () => Promise<void>;
  onCopy: () => Promise<void>;
  onFind?: () => void;
  onOpenFolder: () => Promise<void>;
  hasSummary: boolean;
}

export function SummaryUpdaterButtonGroup({
  isSaving,
  isDirty,
  onSave,
  onCopy,
  hasSummary
}: SummaryUpdaterButtonGroupProps) {
  const t = useTranslations('meetingDetails');
  const tc = useTranslations('common');

  const quiet =
    'flex h-8 shrink-0 items-center justify-center gap-1.5 rounded-lg px-2 text-sm text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)] disabled:opacity-40 disabled:hover:bg-transparent';

  return (
    <div className="flex items-center gap-0.5">
      {/* Named only while there is something to save, so it is noticed then. */}
      <button
        type="button"
        className={`${quiet} ${isDirty ? 'text-[var(--af-accent)]' : 'w-8 px-0'}`}
        title={isSaving ? t('summarySavingTooltip') : t('summarySaveTooltip')}
        aria-label={tc('save')}
        onClick={() => {
          onSave();
        }}
        disabled={isSaving}
      >
        {isSaving ? <Loader2 size={16} className="animate-spin" /> : <Save size={16} />}
        {isDirty && <span>{isSaving ? t('summarySavingLabel') : tc('save')}</span>}
      </button>

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

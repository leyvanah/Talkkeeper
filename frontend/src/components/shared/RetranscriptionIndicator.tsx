'use client';

/**
 * The retranscription job as seen from anywhere in the application.
 *
 * Shown only when nothing else is already displaying the job, so opening the
 * dialog that owns it does not leave two progress bars on screen. Its whole
 * purpose is the case where the owner closed that dialog and walked around the
 * app while an hour of audio is being re-read.
 */

import { useTranslations } from 'next-intl';
import { Loader2, Sparkles } from 'lucide-react';
import { useRetranscription } from '@/contexts/RetranscriptionContext';

export function RetranscriptionIndicator() {
  const t = useTranslations('app');
  const { job, needsAmbientIndicator } = useRetranscription();

  if (!job || !needsAmbientIndicator) return null;

  // Never a bare zero: a bar with nothing in it reads as "stuck", not "starting"
  const visibleProgress = Math.max(4, Math.min(100, job.progress));

  return (
    <div
      role="status"
      aria-live="polite"
      className="fixed bottom-4 right-4 z-40 w-[min(24rem,calc(100vw-2rem))] rounded-xl border border-[var(--af-border)] bg-[var(--af-panel)] p-4 shadow-2xl"
    >
      <div className="flex items-start gap-3">
        <div className="mt-0.5 rounded-lg bg-blue-500/10 p-2 text-blue-400">
          <Sparkles size={17} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-3">
            <p className="text-sm font-semibold text-[var(--af-text)]">
              {t('postCallImprovingTranscript')}
            </p>
            <span className="shrink-0 text-xs tabular-nums text-[var(--af-text-3)]">
              {visibleProgress}%
            </span>
          </div>
          <div className="mt-1 flex items-center gap-2 text-xs text-[var(--af-text-2)]">
            <Loader2 size={13} className="shrink-0 animate-spin text-blue-400" />
            <span className="truncate">{job.message}</span>
          </div>
          <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-[var(--af-panel-2)]">
            <div
              className="h-full rounded-full bg-blue-500 transition-[width] duration-300"
              style={{ width: `${visibleProgress}%` }}
            />
          </div>
          <p className="mt-2 text-[11px] text-[var(--af-text-3)]">
            {t('postCallKeepReviewing')}
          </p>
        </div>
      </div>
    </div>
  );
}

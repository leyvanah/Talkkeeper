'use client';

/**
 * The retranscription job as seen from anywhere in the application.
 *
 * Shown only when nothing else is already displaying the job, so opening the
 * dialog that owns it does not leave two progress bars on screen. Its whole
 * purpose is the case where the owner closed that dialog and walked around the
 * app while an hour of audio is being re-read.
 */

import { useState } from 'react';
import { useTranslations } from 'next-intl';
import { Loader2, Sparkles, X } from 'lucide-react';
import { toast } from 'sonner';
import { useRetranscription } from '@/contexts/RetranscriptionContext';

export function RetranscriptionIndicator() {
  const t = useTranslations('app');
  const td = useTranslations('meetingDetails');
  const { job, needsAmbientIndicator, cancel } = useRetranscription();
  const [stopping, setStopping] = useState(false);

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
            <div className="flex shrink-0 items-center gap-2">
              <span className="text-xs tabular-nums text-[var(--af-text-3)]">
                {visibleProgress}%
              </span>
              {/* The work runs with no window of its own, so this is the only
                  place it can be stopped from. */}
              <button
                type="button"
                aria-label={td('retranscribeCancel')}
                title={td('retranscribeCancel')}
                disabled={stopping}
                onClick={() => {
                  setStopping(true);
                  void cancel()
                    .then(() => toast.info(td('retranscribeCancelled')))
                    .finally(() => setStopping(false));
                }}
                className="rounded-md p-1 text-[var(--af-text-3)] transition-colors hover:bg-[var(--af-panel-2)] hover:text-[var(--af-text)] disabled:opacity-50"
              >
                <X size={14} />
              </button>
            </div>
          </div>
          <div className="mt-1 flex items-center gap-2 text-xs text-[var(--af-text-2)]">
            <Loader2 size={13} className="shrink-0 animate-spin text-blue-400" />
            {/* A job picked up from the backend has said nothing yet, and a
                long recording is decoded in silence before it does. */}
            <span className="truncate">{job.message || t('postCallPreparing')}</span>
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

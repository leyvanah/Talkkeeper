'use client';

/**
 * The retranscription job as seen from anywhere in the application.
 *
 * It lives in the window's header: a short note with the percentage, and the
 * progress as a thin line along the header's lower edge. The work can take
 * the better part of an hour, so it neither floats over the page nor takes a
 * row of its own for that long. What the work is doing right now is in the
 * note's tooltip.
 *
 * Shown only when nothing else is already displaying the job, so opening the
 * dialog that owns it does not leave two progress bars on screen.
 */

import { useState } from 'react';
import { useTranslations } from 'next-intl';
import { Loader2, Pause, X } from 'lucide-react';
import { toast } from 'sonner';
import { useRetranscription } from '@/contexts/RetranscriptionContext';

export function RetranscriptionIndicator() {
  const t = useTranslations('app');
  const td = useTranslations('meetingDetails');
  const { job, ambientStatus, needsAmbientIndicator, cancel } = useRetranscription();
  const [stopping, setStopping] = useState(false);

  if (!ambientStatus || !needsAmbientIndicator) return null;
  // Only a real retranscription can be stopped from here; a wider workflow
  // reports through this strip but is not cancellable by it.
  const stoppable = job !== null;

  // Never a bare zero: a bar with nothing in it reads as "stuck", not "starting"
  const visibleProgress = Math.max(2, Math.min(100, ambientStatus.progress));

  // The job steps aside while a recording is on, and says so.
  const paused = job?.stage === 'paused';
  const detail = paused ? t('postCallPausedDetail') : ambientStatus.message || t('postCallPreparing');

  return (
    <>
      <div
        role="status"
        aria-live="polite"
        title={detail}
        className="flex shrink-0 items-center gap-1.5 rounded-md px-1.5 text-xs text-[var(--af-text-2)]"
      >
        {paused
          ? <Pause size={13} className="shrink-0 text-[var(--af-text-3)]" />
          : <Loader2 size={13} className="shrink-0 animate-spin text-[var(--af-text-3)]" />}
        <span className="hidden max-w-[14rem] truncate lg:inline">
          {paused ? t('postCallPaused') : t('postCallImprovingTranscript')}
        </span>
        <span className="tabular-nums text-[var(--af-text-3)]">{Math.round(ambientStatus.progress)}%</span>
        <span className="sr-only">{detail}</span>
        {/* The work runs with no window of its own, so this is the only place
            it can be stopped from. */}
        {stoppable && <button
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
          className="shrink-0 rounded p-0.5 text-[var(--af-text-3)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] disabled:opacity-50"
        >
          <X size={13} />
        </button>}
      </div>
      {/* Along the header's lower edge, over its border. */}
      <div aria-hidden className="pointer-events-none absolute inset-x-0 bottom-0 h-0.5">
        <div
          className="h-full bg-[var(--af-accent)] transition-[width] duration-300"
          style={{ width: `${visibleProgress}%` }}
        />
      </div>
    </>
  );
}

'use client';

/**
 * The retranscription job as seen from anywhere in the application.
 *
 * A strip along the bottom rather than a card floating over the corner: the
 * work can take the better part of an hour, and for that long it must not sit
 * on top of whatever is underneath it. Being part of the layout, it takes its
 * own height and gives it back when the work ends.
 *
 * Shown only when nothing else is already displaying the job, so opening the
 * dialog that owns it does not leave two progress bars on screen.
 */

import { useState } from 'react';
import { useTranslations } from 'next-intl';
import { Loader2, Sparkles, X } from 'lucide-react';
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

  return (
    <div
      role="status"
      aria-live="polite"
      className="relative shrink-0 border-t border-[var(--af-border)] bg-[var(--af-panel)]"
    >
      {/* The progress itself rides the top edge, so the strip stays one line */}
      <div className="absolute inset-x-0 top-0 h-0.5 bg-[var(--af-panel-2)]">
        <div
          className="h-full bg-blue-500 transition-[width] duration-300"
          style={{ width: `${visibleProgress}%` }}
        />
      </div>

      <div className="flex items-center gap-3 px-4 py-2">
        <Sparkles size={14} className="shrink-0 text-blue-400" />
        <span className="shrink-0 text-xs font-medium text-[var(--af-text)]">
          {t('postCallImprovingTranscript')}
        </span>
        <Loader2 size={12} className="shrink-0 animate-spin text-blue-400" />
        {/* A job picked up from the backend has said nothing yet, and a long
            recording is decoded in silence before it does. */}
        <span className="min-w-0 flex-1 truncate text-xs text-[var(--af-text-2)]">
          {ambientStatus.message || t('postCallPreparing')}
        </span>
        <span className="shrink-0 text-xs tabular-nums text-[var(--af-text-3)]">
          {Math.round(ambientStatus.progress)}%
        </span>
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
          className="shrink-0 rounded-md p-1 text-[var(--af-text-3)] transition-colors hover:bg-[var(--af-panel-2)] hover:text-[var(--af-text)] disabled:opacity-50"
        >
          <X size={14} />
        </button>}
      </div>
    </div>
  );
}

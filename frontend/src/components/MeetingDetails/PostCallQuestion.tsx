'use client';

/**
 * The question the work after a recording asks: how many voices to expect,
 * or, after a failure, whether to try again or keep the live transcript.
 * The work itself lives in `PostCallProvider`; this is only what it shows.
 */

import { useState } from 'react';
import { useTranslations } from 'next-intl';
import { Users } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import type { PostCallRun } from '@/contexts/PostCallContext';

/** How many voices to expect, or what to do after a failure. */
export function PostCallQuestion({
  run,
  onRun,
  onSkipEnhancement,
  onUseLive,
}: {
  run: PostCallRun;
  onRun: (count: number | null) => void;
  onSkipEnhancement: (count: number | null) => void;
  onUseLive: () => void;
}) {
  const t = useTranslations('app');
  const [speakerCount, setSpeakerCount] = useState('2');
  const [autoDetect, setAutoDetect] = useState(false);
  const [countError, setCountError] = useState<string | null>(null);

  const selectedCount = (): number | null | undefined => {
    if (autoDetect) return null;
    const count = Number(speakerCount);
    if (!Number.isInteger(count) || count < 1 || count > 20) {
      setCountError(t('postCallSpeakerCountError'));
      return undefined;
    }
    return count;
  };

  const handleOpenChange = (open: boolean) => {
    if (open) return;
    if (run.stage === 'prompt') {
      const count = selectedCount();
      if (count !== undefined) onSkipEnhancement(count);
    } else {
      onUseLive();
    }
  };

  const error = countError ?? run.error;

  return (
    <Dialog open onOpenChange={handleOpenChange}>
      <DialogContent
        aria-describedby="post-call-processing-description"
        className="sm:max-w-md"
        onEscapeKeyDown={(event) => event.preventDefault()}
        onPointerDownOutside={(event) => event.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Users size={18} className="text-blue-400" />
            {t('postCallHowMany')}
          </DialogTitle>
          <DialogDescription id="post-call-processing-description">
            {t('postCallHowManyDescription')}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          <div className={run.singleRemoteSpeaker ? 'hidden' : 'grid grid-cols-4 gap-2'}>
            {[1, 2, 3, 4, 5, 6, 7, 8].map((count) => (
              <Button
                key={count}
                type="button"
                variant={!autoDetect && speakerCount === String(count) ? 'default' : 'outline'}
                onClick={() => {
                  setSpeakerCount(String(count));
                  setAutoDetect(false);
                  setCountError(null);
                }}
              >
                {count}
              </Button>
            ))}
            <Button
              type="button"
              className="col-span-4"
              variant={autoDetect ? 'default' : 'outline'}
              onClick={() => {
                setAutoDetect(true);
                setCountError(null);
              }}
            >
              {t('postCallAutoDetect')}
            </Button>
          </div>
          <input
            type="number"
            min={1}
            max={20}
            value={autoDetect ? '' : speakerCount}
            placeholder={autoDetect ? t('postCallSpeakersAutoPlaceholder') : undefined}
            onFocus={() => setAutoDetect(false)}
            onChange={(event) => {
              setSpeakerCount(event.target.value);
              setAutoDetect(false);
            }}
            className={`${run.singleRemoteSpeaker ? 'hidden' : ''} w-full rounded-md border border-[var(--af-border)] bg-[var(--af-panel-2)] px-3 py-2 text-sm text-[var(--af-text)] outline-none focus:ring-2 focus:ring-blue-500`}
            aria-label={t('postCallSpeakerCountAria')}
          />
          {error && <p className="text-sm text-red-400">{error}</p>}
        </div>

        <DialogFooter>
          {run.stage === 'error' && (
            <Button type="button" variant="outline" onClick={onUseLive}>
              {t('postCallUseLiveTranscript')}
            </Button>
          )}
          <Button
            type="button"
            onClick={() => {
              const count = selectedCount();
              if (count !== undefined) onRun(count);
            }}
          >
            {run.stage === 'error' ? t('postCallRetry') : t('postCallEnhance')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

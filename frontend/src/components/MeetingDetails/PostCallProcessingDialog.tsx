"use client";

/**
 * Owns the post-recording handoff. This must remain a single ordered workflow:
 * speaker count choice -> source-track retranscription -> refetch ->
 * diarization -> refetch -> summary gate. Starting summary or diarization
 * elsewhere races the transactional transcript replacement.
 *
 * Retranscription is event-driven because the Tauri start command returns after
 * spawning its native task. Listeners therefore register before invoke and are
 * meeting-ID filtered. The two refresh failures are intentionally distinct:
 * retrying the first must still run diarization, while retrying the second may
 * complete immediately. Audio-disabled meetings can explicitly continue with
 * their live transcript so the summary stage is never permanently blocked.
 */

import { useEffect, useRef, useState } from 'react';
import { useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import { Loader2, Sparkles, Users } from 'lucide-react';
import { toast } from 'sonner';
import { useConfig } from '@/contexts/ConfigContext';
import { useRetranscription, ENHANCEMENT_STALLED } from '@/contexts/RetranscriptionContext';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import type { RawModelInfo } from '@/hooks/useTranscriptionModels';
import { isVisibleParakeetModel } from '@/lib/parakeet';
import { externalSttLabel, type ExternalSttConfig } from '@/components/ExternalSttSettings';
import { GIGAAM_MODEL_NAME, type GigaamModelStatus } from '@/components/GigaamModelManager';

type Stage = 'idle' | 'prompt' | 'enhancing' | 'diarizing' | 'refreshing' | 'error';
type FailedStage = 'enhancing' | 'diarizing' | 'pre-diarization-refresh' | 'post-diarization-refresh';




interface ModelChoice {
  provider: 'whisper' | 'parakeet' | 'gigaam' | 'externalStt';
  name: string;
}

interface PostCallTranscriptConfig {
  provider: 'live' | 'whisper' | 'parakeet';
  model: string;
}

async function resolveEnhancementModel(
  configuredProvider?: string,
  configuredModel?: string,
): Promise<ModelChoice> {
  const [whisperModels, parakeetModels] = await Promise.all([
    invoke<RawModelInfo[]>('whisper_get_available_models').catch(() => []),
    invoke<RawModelInfo[]>('parakeet_get_available_models').catch(() => []),
  ]);
  const available: ModelChoice[] = [
    ...whisperModels
      .filter((model) => model.status === 'Available')
      .map((model) => ({ provider: 'whisper' as const, name: model.name })),
    ...parakeetModels
      .filter((model) => model.status === 'Available' && isVisibleParakeetModel(model.name))
      .map((model) => ({ provider: 'parakeet' as const, name: model.name })),
  ];
  // GigaAM can enhance as well, once its model is downloaded
  const gigaam = await invoke<GigaamModelStatus>('gigaam_get_model_status').catch(() => null);
  if (gigaam?.installed) {
    available.push({ provider: 'gigaam' as const, name: GIGAAM_MODEL_NAME });
  }

  // The external service has no downloaded model, but it can enhance too
  const externalConfig = await invoke<ExternalSttConfig>('api_get_external_stt_config')
    .catch(() => null);
  if (externalConfig?.url.trim()) {
    available.push({ provider: 'externalStt' as const, name: externalSttLabel(externalConfig) });
  }
  const normalizedProvider = configuredProvider === 'localWhisper'
    ? 'whisper'
    : configuredProvider;
  if (normalizedProvider === 'gigaam') {
    const local = available.find((model) => model.provider === 'gigaam');
    if (local) return local;
    throw new Error('The GigaAM model is not downloaded for enhancement.');
  }
  if (normalizedProvider === 'externalStt') {
    const external = available.find((model) => model.provider === 'externalStt');
    if (external) return external;
    throw new Error('The external speech service is not configured for enhancement.');
  }
  const configured = available.find(
    (model) => model.provider === normalizedProvider && model.name === configuredModel,
  );
  if (configured) return configured;
  if (normalizedProvider === 'whisper' || normalizedProvider === 'parakeet') {
    const sameProvider = available.find((model) => model.provider === normalizedProvider);
    if (sameProvider) return sameProvider;
    throw new Error(`No downloaded ${normalizedProvider} model is available for enhancement.`);
  }
  const localDefault = available.find((model) => model.provider === 'parakeet') ?? available[0];
  if (localDefault) return localDefault;
  throw new Error('No downloaded transcription model is available for post-call enhancement.');
}


export function PostCallProcessingDialog({
  enabled,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
  onComplete,
}: {
  enabled: boolean;
  meetingId: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;
  onComplete: () => void;
}) {
  const t = useTranslations('app');
  // The stall is ours to name; everything else already arrives as a message.
  const describeFailure = (cause: unknown) => {
    const message = cause instanceof Error ? cause.message : String(cause);
    return message === ENHANCEMENT_STALLED ? t('postCallTimedOut') : message;
  };
  const { selectedLanguage, transcriptModelConfig } = useConfig();
  const { job, start: startRetranscription, setAmbientStep } = useRetranscription();
  const [stage, setStage] = useState<Stage>('idle');
  const [speakerCount, setSpeakerCount] = useState('2');
  const [autoDetectSpeakers, setAutoDetectSpeakers] = useState(false);
  // One conversation partner: the capture channels already say who is who
  const [singleRemoteSpeaker, setSingleRemoteSpeaker] = useState(true);
  const [progress, setProgress] = useState(0);
  const [message, setMessage] = useState(() => t('postCallPreparing'));
  const [error, setError] = useState<string | null>(null);
  const [failedStage, setFailedStage] = useState<FailedStage | null>(null);
  const initializedMeetingRef = useRef<string | null>(null);
  const activeStageRef = useRef<FailedStage>('enhancing');
  const skippedEnhancementRef = useRef(false);

  const storageKey = `post-call-processing:${meetingId}`;

  // The card below covers the whole workflow, retranscription included, so it
  // mirrors the shared job while that stage runs rather than listening itself.
  useEffect(() => {
    if (job?.meetingId !== meetingId) return;
    setProgress(job.progress);
    setMessage(job.message);
  }, [job, meetingId]);

  useEffect(() => {
    invoke<{ single_remote_speaker?: boolean }>('get_recording_preferences')
      .then((prefs) => setSingleRemoteSpeaker(prefs.single_remote_speaker !== false))
      .catch((error) => console.error('Failed to read recording preferences:', error));
  }, []);

  useEffect(() => {
    if (!enabled || !meetingId || initializedMeetingRef.current === meetingId) return;
    initializedMeetingRef.current = meetingId;
    const terminalState = sessionStorage.getItem(storageKey);
    if (terminalState === 'completed') {
      onComplete();
      return;
    }
    setStage('prompt');
  }, [enabled, meetingId, onComplete, storageKey]);

  const completeWorkflow = () => {
    sessionStorage.setItem(storageKey, 'completed');
    setStage('idle');
    toast.success(t('postCallComplete'), {
      description: skippedEnhancementRef.current
        ? t('postCallSpeakersRefreshed')
        : t('postCallEnhancedAndRefreshed'),
    });
    onComplete();
  };

  const refreshTranscript = async (
    phase: 'pre-diarization-refresh' | 'post-diarization-refresh',
  ) => {
    activeStageRef.current = phase;
    setStage('refreshing');
    setMessage(t('postCallRefreshing'));
    await onRefetchTranscripts?.();
  };

  const identifySpeakers = async (count: number | null) => {
    if (singleRemoteSpeaker) {
      // Microphone is you, the speakers are them - telling voices apart would
      // only split one person into several
      completeWorkflow();
      return;
    }
    activeStageRef.current = 'diarizing';
    setStage('diarizing');
    setProgress(100);
    setMessage(count === null
      ? t('postCallAutoDetecting')
      : t('postCallIdentifyingSpeakers', { count }));
    await invoke('diarize_meeting', { meetingId, numSpeakers: count });
    await refreshTranscript('post-diarization-refresh');
    completeWorkflow();
  };

  const runWorkflow = async (count: number | null) => {
    if (!meetingFolderPath) {
      throw new Error(t('postCallNoFolder'));
    }

    setError(null);
    setFailedStage(null);
    activeStageRef.current = 'enhancing';
    setStage('enhancing');
    setProgress(0);
    setMessage(t('postCallPreparing'));
    const postCallConfig = await invoke<PostCallTranscriptConfig>('api_get_post_call_transcript_config')
      .catch(() => ({ provider: 'live' as const, model: '' }));
    const useLiveDefault = postCallConfig.provider === 'live';
    const model = await resolveEnhancementModel(
      useLiveDefault ? transcriptModelConfig?.provider : postCallConfig.provider,
      useLiveDefault ? transcriptModelConfig?.model : postCallConfig.model,
    );
    await startRetranscription({
      meetingId,
      meetingFolderPath,
      language: model.provider === 'parakeet' || selectedLanguage === 'auto'
        ? null
        : selectedLanguage || null,
      model: model.name,
      provider: model.provider,
    });
    // Retranscription transactionally replaces the rows. Refresh immediately so
    // a later diarization error can never leave the old live transcript onscreen.
    await refreshTranscript('pre-diarization-refresh');
    await identifySpeakers(count);
  };

  const getSelectedSpeakerCount = (): number | null | undefined => {
    if (autoDetectSpeakers) return null;
    const count = Number(speakerCount);
    if (!Number.isInteger(count) || count < 1 || count > 20) {
      setError(t('postCallSpeakerCountError'));
      return undefined;
    }
    return count;
  };

  const start = async () => {
    const count = getSelectedSpeakerCount();
    if (count === undefined) return;
    try {
      if (failedStage === 'diarizing') {
        await identifySpeakers(count);
      } else if (failedStage === 'pre-diarization-refresh') {
        await refreshTranscript('pre-diarization-refresh');
        await identifySpeakers(count);
      } else if (failedStage === 'post-diarization-refresh') {
        await refreshTranscript('post-diarization-refresh');
        completeWorkflow();
      } else {
        skippedEnhancementRef.current = false;
        await runWorkflow(count);
      }
    } catch (cause) {
      const nextError = describeFailure(cause);
      setFailedStage(activeStageRef.current);
      setError(nextError);
      setStage('error');
      toast.error(t('postCallFailed'), { description: nextError });
    }
  };

  const skipEnhancement = async () => {
    const count = getSelectedSpeakerCount();
    if (count === undefined) return;
    skippedEnhancementRef.current = true;
    setError(null);
    setFailedStage(null);
    try {
      await identifySpeakers(count);
    } catch (cause) {
      const nextError = describeFailure(cause);
      setFailedStage(activeStageRef.current);
      setError(nextError);
      setStage('error');
      toast.error(t('postCallSpeakerFailed'), { description: nextError });
    }
  };

  const continueWithLiveTranscript = async () => {
    try {
      skippedEnhancementRef.current = true;
      await refreshTranscript('post-diarization-refresh');
      completeWorkflow();
      toast.info(t('postCallUsingLive'), {
        description: t('postCallUsingLiveDescription'),
      });
    } catch (cause) {
      const nextError = describeFailure(cause);
      setFailedStage('post-diarization-refresh');
      setError(nextError);
      setStage('error');
    }
  };

  const isWorking = stage === 'enhancing' || stage === 'diarizing' || stage === 'refreshing';

  // No window of its own: the workflow reports through the strip at the foot
  // of the application, which is also where its retranscription stage reports.
  useEffect(() => {
    if (!isWorking) {
      setAmbientStep(null);
      return;
    }
    setAmbientStep({ meetingId, progress, message });
  }, [isWorking, meetingId, progress, message, setAmbientStep]);
  useEffect(() => () => setAmbientStep(null), [setAmbientStep]);
  const visibleProgress = Math.max(4, Math.min(100, progress));

  // DialogContent already renders a compact X button. At the count prompt that
  // X means "keep the live transcript": skip only retranscription, then still
  // run speaker identification and unblock the fresh-summary stage.
  const handleOpenChange = (open: boolean) => {
    if (open || isWorking) return;
    if (stage === 'prompt') {
      void skipEnhancement();
    } else if (stage === 'error') {
      void continueWithLiveTranscript();
    }
  };

  return (
    <>
      <Dialog open={stage === 'prompt' || stage === 'error'} onOpenChange={handleOpenChange}>
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
            <div className={singleRemoteSpeaker ? 'hidden' : 'grid grid-cols-4 gap-2'}>
              {[1, 2, 3, 4, 5, 6, 7, 8].map((count) => (
                <Button
                  key={count}
                  type="button"
                  variant={!autoDetectSpeakers && speakerCount === String(count) ? 'default' : 'outline'}
                  onClick={() => {
                    setSpeakerCount(String(count));
                    setAutoDetectSpeakers(false);
                    setError(null);
                  }}
                >
                  {count}
                </Button>
              ))}
              <Button
                type="button"
                className="col-span-4"
                variant={autoDetectSpeakers ? 'default' : 'outline'}
                onClick={() => {
                  setAutoDetectSpeakers(true);
                  setError(null);
                }}
              >
                {t('postCallAutoDetect')}
              </Button>
            </div>
            <input
              type="number"
              min={1}
              max={20}
              value={autoDetectSpeakers ? '' : speakerCount}
              placeholder={autoDetectSpeakers ? t('postCallSpeakersAutoPlaceholder') : undefined}
              onFocus={() => setAutoDetectSpeakers(false)}
              onChange={(event) => {
                setSpeakerCount(event.target.value);
                setAutoDetectSpeakers(false);
              }}
              className="w-full rounded-md border border-[var(--af-border)] bg-[var(--af-panel-2)] px-3 py-2 text-sm text-[var(--af-text)] outline-none focus:ring-2 focus:ring-blue-500"
              aria-label={t('postCallSpeakerCountAria')}
            />
            {error && <p className="text-sm text-red-400">{error}</p>}
          </div>

          <DialogFooter>
            {stage === 'error' && (
              <Button type="button" variant="outline" onClick={continueWithLiveTranscript}>
                {t('postCallUseLiveTranscript')}
              </Button>
            )}
            <Button type="button" onClick={start}>
              {stage === 'error' ? t('postCallRetry') : t('postCallEnhance')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

    </>
  );
}

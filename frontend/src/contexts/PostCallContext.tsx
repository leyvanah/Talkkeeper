'use client';

/**
 * Owns what happens to a recording after it stops: speaker count choice ->
 * source-track retranscription -> speaker identification -> the summary gate.
 *
 * It used to live in a dialog on the meeting page, so the work belonged to
 * whichever page happened to be showing. Leaving the page mid-way left the
 * steps running in a closure nobody listened to; coming back mounted a second
 * copy that started the whole thing again, was refused by the backend (one
 * retranscription at a time), and in failing took the first copy's waiter with
 * it - so speakers and summary never came at all.
 *
 * So the workflow lives here, above every page, like the retranscription it
 * contains. A page only asks for it to begin (which is idempotent), shows the
 * transcript it produces, and starts the summary once it is done. Work for a
 * second recording waits for the first rather than colliding with it.
 */

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useTranslations } from 'next-intl';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useConfig } from '@/contexts/ConfigContext';
import { useRetranscription, ENHANCEMENT_STALLED } from '@/contexts/RetranscriptionContext';
import { PostCallQuestion } from '@/components/MeetingDetails/PostCallQuestion';
import type { RawModelInfo } from '@/hooks/useTranscriptionModels';
import { isVisibleParakeetModel } from '@/lib/parakeet';
import { externalSttLabel, type ExternalSttConfig } from '@/components/ExternalSttSettings';
import { GIGAAM_MODEL_NAME, type GigaamModelStatus } from '@/components/GigaamModelManager';

export type Stage = 'queued' | 'prompt' | 'enhancing' | 'diarizing' | 'error' | 'done';
export type FailedStage = 'enhancing' | 'diarizing';

export interface PostCallRun {
  meetingId: string;
  folderPath: string | null;
  stage: Stage;
  progress: number;
  message: string;
  error: string | null;
  failedStage: FailedStage | null;
  /** One conversation partner: the capture channels already say who is who. */
  singleRemoteSpeaker: boolean;
}

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

/** Survives a reload of the window; the run itself does not need to. */
const completedKey = (meetingId: string) => `post-call-processing:${meetingId}`;

function readCompleted(meetingId: string): boolean {
  try {
    return sessionStorage.getItem(completedKey(meetingId)) === 'completed';
  } catch {
    return false;
  }
}

interface PostCallContextValue {
  /** Start the work for a freshly stopped recording. Safe to call again. */
  begin: (meetingId: string, folderPath: string | null) => void;
  /**
   * The work for this meeting is under way or finished and its summary has
   * not been taken care of yet — the page should behave as just recorded.
   */
  isPending: (meetingId: string) => boolean;
  /** The transcript is final: the summary may start. */
  isDone: (meetingId: string) => boolean;
  /** The summary has started, or will not: nothing is left to do here. */
  release: (meetingId: string) => void;
  /** Goes up each time the stored transcript of the meeting was replaced. */
  transcriptVersion: (meetingId: string) => number;
}

const PostCallContext = createContext<PostCallContextValue | null>(null);

export function usePostCall(): PostCallContextValue {
  const context = useContext(PostCallContext);
  if (!context) throw new Error('usePostCall must be used inside PostCallProvider');
  return context;
}

export function PostCallProvider({ children }: { children: React.ReactNode }) {
  const t = useTranslations('app');
  const router = useRouter();
  const { selectedLanguage, transcriptModelConfig } = useConfig();
  const { job, start: startRetranscription, setAmbientStep } = useRetranscription();

  const [runs, setRuns] = useState<Record<string, PostCallRun>>({});
  // The async steps read the newest state, not the one they were started with.
  const runsRef = useRef(runs);
  runsRef.current = runs;
  const [versions, setVersions] = useState<Record<string, number>>({});

  // Settings the steps need; kept current for code that outlives a render.
  const configRef = useRef({ selectedLanguage, transcriptModelConfig });
  configRef.current = { selectedLanguage, transcriptModelConfig };

  // One recording's heavy work at a time: the backend runs one retranscription
  // and one diarization, and a second request would only be refused.
  const lockRef = useRef<Promise<void>>(Promise.resolve());
  const exclusive = useCallback(<T,>(work: () => Promise<T>): Promise<T> => {
    const result = lockRef.current.then(work);
    lockRef.current = result.then(() => undefined, () => undefined);
    return result;
  }, []);

  const update = useCallback((meetingId: string, patch: Partial<PostCallRun>) => {
    setRuns((current) => {
      const run = current[meetingId];
      if (!run) return current;
      const next = { ...current, [meetingId]: { ...run, ...patch } };
      runsRef.current = next;
      return next;
    });
  }, []);

  const bumpVersion = useCallback((meetingId: string) => {
    setVersions((current) => ({ ...current, [meetingId]: (current[meetingId] ?? 0) + 1 }));
  }, []);

  const describeFailure = useCallback((cause: unknown) => {
    const message = cause instanceof Error ? cause.message : String(cause);
    return message === ENHANCEMENT_STALLED ? t('postCallTimedOut') : message;
  }, [t]);

  const complete = useCallback((meetingId: string, skippedEnhancement: boolean) => {
    try {
      sessionStorage.setItem(completedKey(meetingId), 'completed');
    } catch {
      /* A reload would then offer the work again; nothing is lost. */
    }
    update(meetingId, { stage: 'done', error: null, failedStage: null });
    // The meeting may not be on screen any more: say so, and offer the way back.
    const showing =
      typeof window !== 'undefined' &&
      window.location.pathname.startsWith('/meeting-details') &&
      new URLSearchParams(window.location.search).get('id') === meetingId;
    toast.success(t('postCallComplete'), {
      description: skippedEnhancement
        ? t('postCallSpeakersRefreshed')
        : t('postCallEnhancedAndRefreshed'),
      action: showing
        ? undefined
        : {
            label: t('postCallOpenMeeting'),
            onClick: () => router.push(`/meeting-details?id=${encodeURIComponent(meetingId)}`),
          },
    });
  }, [router, t, update]);

  const fail = useCallback((meetingId: string, stage: FailedStage, cause: unknown, title: string) => {
    const error = describeFailure(cause);
    update(meetingId, { stage: 'error', error, failedStage: stage });
    toast.error(title, { description: error });
  }, [describeFailure, update]);

  /** Tell the speakers apart, where the capture channels do not already. */
  const identifySpeakers = useCallback(async (meetingId: string, count: number | null) => {
    const run = runsRef.current[meetingId];
    if (!run || run.singleRemoteSpeaker) return;
    update(meetingId, {
      stage: 'diarizing',
      progress: 100,
      message: count === null
        ? t('postCallAutoDetecting')
        : t('postCallIdentifyingSpeakers', { count }),
    });
    // Several people on the far side: they share one track, so the voices
    // have to be told apart by the model, not by the device.
    await invoke('diarize_meeting', { meetingId, numSpeakers: count, method: 'model' });
    bumpVersion(meetingId);
  }, [bumpVersion, t, update]);

  const enhance = useCallback(async (meetingId: string) => {
    const run = runsRef.current[meetingId];
    if (!run?.folderPath) throw new Error(t('postCallNoFolder'));
    update(meetingId, { stage: 'enhancing', progress: 0, message: t('postCallPreparing') });
    const { selectedLanguage: language, transcriptModelConfig: live } = configRef.current;
    const postCallConfig = await invoke<PostCallTranscriptConfig>('api_get_post_call_transcript_config')
      .catch(() => ({ provider: 'live' as const, model: '' }));
    const useLiveDefault = postCallConfig.provider === 'live';
    const model = await resolveEnhancementModel(
      useLiveDefault ? live?.provider : postCallConfig.provider,
      useLiveDefault ? live?.model : postCallConfig.model,
    );
    await startRetranscription({
      meetingId,
      meetingFolderPath: run.folderPath,
      language: model.provider === 'parakeet' || language === 'auto' ? null : language || null,
      model: model.name,
      provider: model.provider,
    });
    // The rows were replaced: whoever shows them reloads now, so a later
    // failure can never leave the live transcript on screen.
    bumpVersion(meetingId);
  }, [bumpVersion, startRetranscription, t, update]);

  /** The whole workflow, or what is left of it after a failure. */
  const proceed = useCallback((meetingId: string, count: number | null, from: FailedStage) =>
    exclusive(async () => {
      let stage: FailedStage = from;
      try {
        if (from === 'enhancing') await enhance(meetingId);
        stage = 'diarizing';
        await identifySpeakers(meetingId, count);
        complete(meetingId, false);
      } catch (cause) {
        fail(meetingId, stage, cause, t('postCallFailed'));
      }
    }), [complete, enhance, exclusive, fail, identifySpeakers, t]);

  const begin = useCallback((meetingId: string, folderPath: string | null) => {
    if (!meetingId || runsRef.current[meetingId] || readCompleted(meetingId)) return;
    const run: PostCallRun = {
      meetingId,
      folderPath,
      stage: 'queued',
      progress: 0,
      message: t('postCallPreparing'),
      error: null,
      failedStage: null,
      singleRemoteSpeaker: true,
    };
    const next = { ...runsRef.current, [meetingId]: run };
    runsRef.current = next;
    setRuns(next);

    void invoke<{ single_remote_speaker?: boolean }>('get_recording_preferences')
      .then((prefs) => prefs.single_remote_speaker !== false)
      .catch((error) => {
        console.error('Failed to read recording preferences:', error);
        return true;
      })
      .then((single) => {
        update(meetingId, { singleRemoteSpeaker: single });
        // One to one: the two capture channels already say who is who, so
        // there is nothing to ask.
        if (single) void proceed(meetingId, 2, 'enhancing');
        else update(meetingId, { stage: 'prompt' });
      });
  }, [proceed, t, update]);

  // The retranscription reports its own progress; the run mirrors it.
  useEffect(() => {
    if (!job) return;
    const run = runsRef.current[job.meetingId];
    if (run?.stage !== 'enhancing') return;
    update(job.meetingId, { progress: job.progress, message: job.message });
  }, [job, update]);

  // The strip at the foot of the application speaks for the running work.
  const working = Object.values(runs).find(
    (run) => run.stage === 'enhancing' || run.stage === 'diarizing',
  );
  useEffect(() => {
    setAmbientStep(
      working
        ? { meetingId: working.meetingId, progress: working.progress, message: working.message }
        : null,
    );
  }, [working?.meetingId, working?.progress, working?.message, setAmbientStep]);

  const isPending = useCallback((meetingId: string) => !!runs[meetingId], [runs]);
  const isDone = useCallback(
    (meetingId: string) => runs[meetingId]?.stage === 'done' || readCompleted(meetingId),
    [runs],
  );
  const release = useCallback((meetingId: string) => {
    setRuns((current) => {
      const run = current[meetingId];
      if (!run || run.stage !== 'done') return current;
      const next = { ...current };
      delete next[meetingId];
      runsRef.current = next;
      return next;
    });
  }, []);
  const transcriptVersion = useCallback((meetingId: string) => versions[meetingId] ?? 0, [versions]);

  const value = useMemo(
    () => ({ begin, isPending, isDone, release, transcriptVersion }),
    [begin, isPending, isDone, release, transcriptVersion],
  );

  // The question and the failure are asked about one recording at a time.
  const asking = Object.values(runs).find((run) => run.stage === 'prompt' || run.stage === 'error');

  return (
    <PostCallContext.Provider value={value}>
      {children}
      {asking && (
        <PostCallQuestion
          key={asking.meetingId}
          run={asking}
          onRun={(count) => {
            if (asking.stage === 'error' && asking.failedStage === 'diarizing') {
              void proceed(asking.meetingId, count, 'diarizing');
            } else {
              void proceed(asking.meetingId, count, 'enhancing');
            }
          }}
          onSkipEnhancement={(count) => {
            // The X at the count question: keep the live transcript, but still
            // tell the speakers apart and let the summary go ahead.
            void exclusive(async () => {
              try {
                await identifySpeakers(asking.meetingId, count);
                complete(asking.meetingId, true);
              } catch (cause) {
                fail(asking.meetingId, 'diarizing', cause, t('postCallSpeakerFailed'));
              }
            });
          }}
          onUseLive={() => {
            complete(asking.meetingId, true);
            toast.info(t('postCallUsingLive'), { description: t('postCallUsingLiveDescription') });
          }}
        />
      )}
    </PostCallContext.Provider>
  );
}

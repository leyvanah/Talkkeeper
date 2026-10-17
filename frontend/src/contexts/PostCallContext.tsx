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
import {
  chooseEnhancementModel,
  PROVIDER_LABELS,
  type EnhancementChoice,
  type ModelChoice,
  type RequestedModel,
} from '@/lib/enhancement-model';
import { planPostCall, type PostCallPlan } from '@/lib/post-call-plan';
import { useSecurity } from '@/contexts/SecurityContext';
import { BackgroundSummary } from '@/components/MeetingDetails/BackgroundSummary';
import { toastFailure } from '@/lib/failure';

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
  /** Lines of the live transcript when the run began; `null` if not counted. */
  lineCount: number | null;
}

/** A failure already worded for the window, shown as it is. */
class ShownFailure extends Error {}

/** Whether the meeting folder holds audio; `null` when it cannot be told. */
async function folderHasAudio(folderPath: string | null): Promise<boolean | null> {
  if (!folderPath) return false;
  // The player's lookup: the same search the refining pass makes for its audio.
  return invoke<string | null>('meeting_playback_file', { meetingFolder: folderPath })
    .then((path) => !!path)
    .catch((error) => {
      console.error('Could not tell whether the recording kept its audio:', error);
      return null;
    });
}

/** Lines in the stored transcript; `null` when they cannot be counted. */
async function transcriptLineCount(meetingId: string): Promise<number | null> {
  return invoke<{ total_count: number }>('api_get_meeting_transcripts', {
    meetingId,
    limit: 1,
    offset: 0,
  })
    .then((page) => page?.total_count ?? 0)
    .catch((error) => {
      console.error('Could not count the transcript lines:', error);
      return null;
    });
}

interface PostCallTranscriptConfig {
  provider: 'live' | 'whisper' | 'parakeet' | 'gigaam';
  model: string;
}

async function resolveEnhancementModel(
  postCall: RequestedModel,
  live: RequestedModel | undefined,
): Promise<EnhancementChoice | null> {
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
  return chooseEnhancementModel(available, postCall, live);
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
  // Meetings processed while their page was not open, waiting for the
  // summary the page would have started; one at a time, like the work.
  const [summaryQueue, setSummaryQueue] = useState<string[]>([]);

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
    if (cause instanceof ShownFailure) return cause.message;
    if (cause instanceof Error && cause.message === ENHANCEMENT_STALLED) return t('postCallTimedOut');
    // The backend's text is English and may name the recording folder, with
    // the Windows user name in it: the window says what to do instead.
    console.error('Post-call processing failed:', cause);
    return t('postCallFailedReason');
  }, [t]);

  /**
   * The transcript is final; remembered across a reload of the window, and
   * across a restart by clearing the meeting's mark in the database.
   */
  const settle = useCallback((meetingId: string) => {
    try {
      sessionStorage.setItem(completedKey(meetingId), 'completed');
    } catch {
      /* A reload would then offer the work again; nothing is lost. */
    }
    void invoke('api_set_post_call_pending', { meetingId, pending: false }).catch((error) =>
      // The next start would offer the work again; nothing is lost.
      console.error('Could not clear the meeting\'s processing mark:', error),
    );
    update(meetingId, { stage: 'done', error: null, failedStage: null });
  }, [update]);

  /**
   * Forgets a finished run that no summary will come to release: without any
   * text there is nothing to summarise, and a run left pending keeps the page
   * behaving as just recorded.
   */
  const drop = useCallback((meetingId: string) => {
    setRuns((current) => {
      if (!current[meetingId]) return current;
      const next = { ...current };
      delete next[meetingId];
      runsRef.current = next;
      return next;
    });
  }, []);

  const complete = useCallback((meetingId: string, skippedEnhancement: boolean) => {
    settle(meetingId);
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
    // On its page the page starts the summary; elsewhere it would wait for
    // the owner to open the meeting.
    if (!showing) {
      setSummaryQueue((queue) => (queue.includes(meetingId) ? queue : [...queue, meetingId]));
    }
  }, [router, settle, t]);

  /**
   * Audio but no live text, and the owner chose not to refine it: nothing to
   * label or summarise. Said as it is rather than as "complete"; the page's
   * "transcribe again" still recovers the text from the audio.
   */
  const keepNoLiveText = useCallback((meetingId: string) => {
    settle(meetingId);
    drop(meetingId);
    toast.info(t('postCallNoLiveText'), { description: t('postCallNoLiveTextDescription') });
  }, [drop, settle, t]);

  /**
   * Nothing to refine: no audio, or no text the summary could start from.
   * Neither is a success to announce.
   */
  const finishUnrefined = useCallback((meetingId: string, plan: Exclude<PostCallPlan, 'refine'>) => {
    settle(meetingId);
    if (plan === 'keep-live') {
      toast.info(t('postCallNoAudio'), { description: t('postCallNoAudioDescription') });
      return;
    }
    toast.warning(t('postCallNothingKept'), { description: t('postCallNothingKeptDescription') });
    drop(meetingId);
  }, [drop, settle, t]);

  const fail = useCallback((meetingId: string, stage: FailedStage, cause: unknown, title: string) => {
    const error = describeFailure(cause);
    update(meetingId, { stage: 'error', error, failedStage: stage });
    // The timeout is ours to explain; any other cause goes to the log only.
    const stalled = cause instanceof Error && cause.message === ENHANCEMENT_STALLED;
    toastFailure(
      title,
      `post-call-${stage}`,
      cause,
      stalled ? { description: t('postCallTimedOut') } : undefined,
    );
  }, [describeFailure, t, update]);

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
    if (!run?.folderPath) throw new ShownFailure(t('postCallNoFolder'));
    update(meetingId, { stage: 'enhancing', progress: 0, message: t('postCallPreparing') });
    const { selectedLanguage: language, transcriptModelConfig: live } = configRef.current;
    const postCallConfig = await invoke<PostCallTranscriptConfig>('api_get_post_call_transcript_config')
      .catch(() => ({ provider: 'live' as const, model: '' }));
    const choice = await resolveEnhancementModel(postCallConfig, live);
    if (!choice) throw new ShownFailure(t('postCallNoModel'));
    const { model, replaced } = choice;
    if (replaced) {
      toast.info(t('postCallModelReplaced', {
        wanted: PROVIDER_LABELS[replaced],
        used: PROVIDER_LABELS[model.provider],
      }));
    }
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
      lineCount: null,
    };
    const next = { ...runsRef.current, [meetingId]: run };
    runsRef.current = next;
    setRuns(next);

    const singleRemote = invoke<{ single_remote_speaker?: boolean }>('get_recording_preferences')
      .then((prefs) => prefs.single_remote_speaker !== false)
      .catch((error) => {
        console.error('Failed to read recording preferences:', error);
        return true;
      });
    // The audio is written before the window hears the recording stopped, so
    // its absence here is final.
    void Promise.all([singleRemote, folderHasAudio(folderPath), transcriptLineCount(meetingId)])
      .then(([single, hasAudio, lineCount]) => {
        update(meetingId, { singleRemoteSpeaker: single, lineCount });
        const plan = planPostCall({ hasAudio, lineCount });
        if (plan !== 'refine') {
          finishUnrefined(meetingId, plan);
          return;
        }
        // One to one: the two capture channels already say who is who, so
        // there is nothing to ask.
        if (single) void proceed(meetingId, 2, 'enhancing');
        else update(meetingId, { stage: 'prompt' });
      });
  }, [finishUnrefined, proceed, t, update]);

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

  // A queue left by a quit, a restart or a reload of the window: a meeting
  // stays marked in the database until its processing settles. Taken up once
  // the archive is open, since the work writes sealed lines.
  const { status: security } = useSecurity();
  const archiveOpen = security?.state === 'unlocked' || security?.state === 'unconfigured';
  const resumedRef = useRef(false);
  useEffect(() => {
    if (!archiveOpen || resumedRef.current) return;
    resumedRef.current = true;
    void invoke<{ meetingId: string; folderPath: string | null }[]>('api_list_post_call_pending')
      .then((waiting) => {
        const left = waiting.filter(({ meetingId }) => {
          if (!readCompleted(meetingId)) return !runsRef.current[meetingId];
          // Settled before a reload whose mark did not clear.
          void invoke('api_set_post_call_pending', { meetingId, pending: false }).catch(() => {});
          return false;
        });
        if (left.length === 0) return;
        toast.info(t('postCallResuming', { count: left.length }));
        for (const { meetingId, folderPath } of left) begin(meetingId, folderPath);
      })
      .catch((error) => console.error('Could not read the recordings waiting for processing:', error));
  }, [archiveOpen, begin, t]);

  const value = useMemo(
    () => ({ begin, isPending, isDone, release, transcriptVersion }),
    [begin, isPending, isDone, release, transcriptVersion],
  );

  // The answer closes the question at once. The work itself may have to wait
  // for another recording's to finish; until it starts, the dialog used to
  // stay open with nothing moving — and each further click queued the same
  // work again.
  const answered = (meetingId: string) => {
    update(meetingId, { stage: 'queued', error: null, message: t('postCallWaiting') });
    if (working && working.meetingId !== meetingId) toast.info(t('postCallWaiting'));
  };

  // The question and the failure are asked about one recording at a time.
  const asking = Object.values(runs).find((run) => run.stage === 'prompt' || run.stage === 'error');

  return (
    <PostCallContext.Provider value={value}>
      {children}
      {summaryQueue[0] && (
        <BackgroundSummary
          key={summaryQueue[0]}
          meetingId={summaryQueue[0]}
          onDone={() => setSummaryQueue((queue) => queue.slice(1))}
        />
      )}
      {asking && (
        <PostCallQuestion
          key={asking.meetingId}
          run={asking}
          onRun={(count) => {
            const from = asking.stage === 'error' && asking.failedStage === 'diarizing' ? 'diarizing' : 'enhancing';
            answered(asking.meetingId);
            void proceed(asking.meetingId, count, from);
          }}
          onSkipEnhancement={(count) => {
            // The X at the count question: keep the live transcript, but still
            // tell the speakers apart and let the summary go ahead. A recording
            // made while another was being refined has no live text: there is
            // nothing to label or summarise, and "complete" would be untrue.
            if (asking.lineCount === 0) {
              keepNoLiveText(asking.meetingId);
              return;
            }
            answered(asking.meetingId);
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
            if (asking.lineCount === 0) {
              keepNoLiveText(asking.meetingId);
              return;
            }
            complete(asking.meetingId, true);
            toast.info(t('postCallUsingLive'), { description: t('postCallUsingLiveDescription') });
          }}
        />
      )}
    </PostCallContext.Provider>
  );
}

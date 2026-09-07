'use client';

/**
 * Owns the one retranscription the application can be running.
 *
 * The backend keeps a single global flag with an RAII guard: there is at most
 * one such job, ever, whichever screen asked for it. The frontend used to
 * disagree - two dialogs each registered their own listeners, kept their own
 * progress and raced to report the same events. Worse, the manual dialog only
 * listened while it was open, so closing it would have orphaned the job; that
 * is why closing was forbidden mid-run, which locked the owner out of the app
 * for as long as the work took. An hour of recording takes the better part of
 * an hour to re-transcribe.
 *
 * So the job lives here, above every page: listeners stay mounted for the
 * session, progress survives navigation, and a dialog is only ever a view onto
 * it. Closing a view stops showing the work, never the work itself.
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
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

export interface RetranscriptionProgress {
  meeting_id: string;
  stage?: string;
  progress_percentage: number;
  message: string;
}

interface RetranscriptionResult {
  meeting_id: string;
  segments_count: number;
  duration_seconds: number;
}

interface RetranscriptionError {
  meeting_id: string;
  error: string;
}

export interface RetranscriptionStartParams {
  meetingId: string;
  meetingFolderPath: string;
  language: string | null;
  model: string | null;
  provider: string | null;
  vocabularyTerms?: string | null;
  vocabularyScope?: string | null;
}

export interface RetranscriptionJob {
  meetingId: string;
  progress: number;
  message: string;
  stage?: string;
}

/**
 * Raised when the backend stopped reporting progress. Named rather than
 * spelled out so the reader sees it in their own language.
 */
export const ENHANCEMENT_STALLED = 'enhancement-stalled';

/**
 * How the work is judged: not by how long it takes, but by whether it is still
 * moving. An hour of recording is two hours of audio - both source tracks - so
 * any fixed budget large enough for a long session is useless as a stall
 * detector, and any budget small enough to detect a stall cancels honest work.
 * The backend reports progress through decoding, resampling, voice detection
 * and every transcribed segment, so the longest legitimate silence is about a
 * minute; five is a stall.
 */
const STALL_TIMEOUT_MS = 5 * 60 * 1000;

interface RetranscriptionContextValue {
  /** The job in flight, or null. */
  job: RetranscriptionJob | null;
  /** True while that job belongs to this meeting. */
  isRunningFor: (meetingId: string) => boolean;
  /**
   * Start a job and settle when it finishes. Rejects with the backend's error,
   * or with `ENHANCEMENT_STALLED` if progress stops arriving.
   */
  start: (params: RetranscriptionStartParams) => Promise<RetranscriptionResult>;
  /** Ask the backend to stop, and wait for it to actually let go. */
  cancel: () => Promise<void>;
  /**
   * True while nothing on screen is already showing the job, so the ambient
   * indicator knows whether it is needed.
   */
  needsAmbientIndicator: boolean;
  /** Called by a view that shows the job itself, while it is doing so. */
  registerJobView: () => () => void;
}

const RetranscriptionContext = createContext<RetranscriptionContextValue | null>(null);

export function useRetranscription(): RetranscriptionContextValue {
  const context = useContext(RetranscriptionContext);
  if (!context) {
    throw new Error('useRetranscription must be used inside RetranscriptionProvider');
  }
  return context;
}

export function RetranscriptionProvider({ children }: { children: React.ReactNode }) {
  const [job, setJob] = useState<RetranscriptionJob | null>(null);

  // The promise handed to whoever started the job. Kept in a ref because the
  // native listeners are registered once and must reach the current waiter.
  const waiterRef = useRef<{
    meetingId: string;
    resolve: (result: RetranscriptionResult) => void;
    reject: (error: unknown) => void;
  } | null>(null);
  const stallTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const clearStallTimer = useCallback(() => {
    if (stallTimerRef.current) {
      clearTimeout(stallTimerRef.current);
      stallTimerRef.current = undefined;
    }
  }, []);

  const settle = useCallback((
    meetingId: string,
    outcome: { result: RetranscriptionResult } | { error: unknown },
  ) => {
    clearStallTimer();
    const waiter = waiterRef.current;
    setJob((current) => (current?.meetingId === meetingId ? null : current));
    if (!waiter || waiter.meetingId !== meetingId) return;
    waiterRef.current = null;
    if ('error' in outcome) waiter.reject(outcome.error);
    else waiter.resolve(outcome.result);
  }, [clearStallTimer]);

  /** Stop the backend and wait for it to release the global flag. */
  const cancel = useCallback(async () => {
    clearStallTimer();
    await invoke('cancel_retranscription_command').catch(() => undefined);
    for (let attempt = 0; attempt < 60; attempt += 1) {
      const active = await invoke<boolean>('is_retranscription_in_progress_command')
        .catch(() => false);
      if (!active) break;
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
  }, [clearStallTimer]);

  const armStallTimer = useCallback((meetingId: string) => {
    clearStallTimer();
    stallTimerRef.current = setTimeout(() => {
      void (async () => {
        await cancel();
        settle(meetingId, { error: new Error(ENHANCEMENT_STALLED) });
      })();
    }, STALL_TIMEOUT_MS);
  }, [cancel, clearStallTimer, settle]);

  // Registered once for the provider's lifetime, so no event can fall into a
  // resubscribe gap and no navigation can orphan a running job.
  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    let disposed = false;

    const attach = (unlisten: UnlistenFn) => {
      if (disposed) unlisten();
      else unlisteners.push(unlisten);
    };

    void (async () => {
      attach(await listen<RetranscriptionProgress>('retranscription-progress', (event) => {
        const { meeting_id: meetingId, progress_percentage: progress, message, stage } =
          event.payload;
        armStallTimer(meetingId);
        setJob({ meetingId, progress, message, stage });
      }));
      attach(await listen<RetranscriptionResult>('retranscription-complete', (event) => {
        settle(event.payload.meeting_id, { result: event.payload });
      }));
      attach(await listen<RetranscriptionError>('retranscription-error', (event) => {
        settle(event.payload.meeting_id, { error: new Error(event.payload.error) });
      }));
    })();

    return () => {
      disposed = true;
      unlisteners.splice(0).forEach((unlisten) => unlisten());
    };
  }, [armStallTimer, settle]);

  useEffect(() => clearStallTimer, [clearStallTimer]);

  // A window can open, or be reloaded, while work is already under way - and
  // the first minutes of a long recording are decoded in silence, with no
  // progress to announce it. Without asking, the screen would show nothing
  // running and offer no way to stop it.
  useEffect(() => {
    let cancelled = false;
    void invoke<{ in_progress: boolean; meeting_id: string | null }>(
      'retranscription_status_command',
    )
      .then((status) => {
        if (cancelled || !status.in_progress || !status.meeting_id) return;
        setJob((current) => current ?? {
          meetingId: status.meeting_id as string,
          progress: 0,
          message: '',
        });
      })
      .catch(() => undefined);
    return () => { cancelled = true; };
  }, []);

  const start = useCallback(async (params: RetranscriptionStartParams) => {
    const {
      meetingId,
      meetingFolderPath,
      language,
      model,
      provider,
      vocabularyTerms = null,
      vocabularyScope = null,
    } = params;

    const completion = new Promise<RetranscriptionResult>((resolve, reject) => {
      waiterRef.current = { meetingId, resolve, reject };
    });
    setJob({ meetingId, progress: 0, message: '' });
    armStallTimer(meetingId);

    try {
      await invoke('start_retranscription_command', {
        meetingId,
        meetingFolderPath,
        language,
        model,
        provider,
        vocabularyTerms,
        vocabularyScope,
      });
    } catch (error) {
      settle(meetingId, { error });
    }
    return completion;
  }, [armStallTimer, settle]);

  const isRunningFor = useCallback(
    (meetingId: string) => job?.meetingId === meetingId,
    [job],
  );

  const [viewerCount, setViewerCount] = useState(0);
  const registerJobView = useCallback(() => {
    setViewerCount((count) => count + 1);
    return () => setViewerCount((count) => Math.max(0, count - 1));
  }, []);

  const value = useMemo(
    () => ({
      job,
      isRunningFor,
      start,
      cancel,
      needsAmbientIndicator: job !== null && viewerCount === 0,
      registerJobView,
    }),
    [job, isRunningFor, start, cancel, viewerCount, registerJobView],
  );

  return (
    <RetranscriptionContext.Provider value={value}>
      {children}
    </RetranscriptionContext.Provider>
  );
}

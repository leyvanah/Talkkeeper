"use client";

/**
 * The player at the foot of the transcript column.
 *
 * It owns the playhead. `currentTime` moves every animation frame, so keeping
 * it here means only this bar re-renders at that rate; the transcript is told
 * only which line is being spoken, which changes a few times a minute.
 */

import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslations } from 'next-intl';
import { Pause, Play } from 'lucide-react';
import { useAudioPlayer } from '@/hooks/useAudioPlayer';
import { TranscriptSegmentData } from '@/types';

export interface RecordingPlayerHandle {
  /** Move the playhead, as clicking a line in the transcript does. */
  seek: (seconds: number) => void;
}

interface RecordingPlayerProps {
  /** Meeting folder holding the recording; `null` while it is unknown. */
  meetingFolderPath?: string | null;
  /** Lines in the order they were spoken, used to say which one is current. */
  segments: TranscriptSegmentData[];
  /** Called when the spoken line changes, including to nothing. */
  onActiveSegmentChange?: (segmentId: string | null) => void;
  /** Called when playback starts or stops, so the transcript can follow along. */
  onPlayingChange?: (isPlaying: boolean) => void;
}

/** `H:MM:SS` past an hour, `MM:SS` before it. */
function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '0:00';
  const whole = Math.floor(seconds);
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const secs = whole % 60;
  const padded = `${minutes.toString().padStart(hours > 0 ? 2 : 1, '0')}:${secs
    .toString()
    .padStart(2, '0')}`;
  return hours > 0 ? `${hours}:${padded}` : padded;
}

/**
 * The line being spoken at `time`: the last one that has started. A line keeps
 * the playhead until the next one begins, so the gaps between turns do not
 * leave the transcript unmarked.
 */
function activeSegmentAt(segments: TranscriptSegmentData[], time: number): string | null {
  let active: string | null = null;
  for (const segment of segments) {
    if ((segment.timestamp ?? 0) > time + 0.001) break;
    active = segment.id;
  }
  return active;
}

export const RecordingPlayer = forwardRef<RecordingPlayerHandle, RecordingPlayerProps>(
  function RecordingPlayer(
    { meetingFolderPath, segments, onActiveSegmentChange, onPlayingChange },
    ref,
  ) {
    const t = useTranslations('meetingDetails');
    const [audioPath, setAudioPath] = useState<string | null>(null);
    const [lookedUp, setLookedUp] = useState(false);
    const { isPlaying, currentTime, duration, error, play, pause, seek } = useAudioPlayer(audioPath);

    useEffect(() => {
      let cancelled = false;
      setAudioPath(null);
      setLookedUp(false);
      if (!meetingFolderPath) {
        setLookedUp(true);
        return;
      }
      invoke<string | null>('meeting_playback_file', { meetingFolder: meetingFolderPath })
        .then((path) => {
          if (!cancelled) setAudioPath(path ?? null);
        })
        .catch((cause) => {
          console.warn('No recording to play for this meeting:', cause);
        })
        .finally(() => {
          if (!cancelled) setLookedUp(true);
        });
      return () => {
        cancelled = true;
      };
    }, [meetingFolderPath]);

    useImperativeHandle(ref, () => ({ seek }), [seek]);

    // Ordered once here so the search for the current line is a walk forward.
    const ordered = useMemo(
      () => [...segments].sort((a, b) => (a.timestamp ?? 0) - (b.timestamp ?? 0)),
      [segments],
    );

    const activeId = useMemo(
      () => (isPlaying || currentTime > 0 ? activeSegmentAt(ordered, currentTime) : null),
      [ordered, currentTime, isPlaying],
    );

    // Report the line, not the time: the transcript should re-render when the
    // speaker moves on, not sixty times a second.
    const reportedRef = useRef<string | null>(null);
    useEffect(() => {
      if (reportedRef.current === activeId) return;
      reportedRef.current = activeId;
      onActiveSegmentChange?.(activeId);
    }, [activeId, onActiveSegmentChange]);

    useEffect(() => {
      onPlayingChange?.(isPlaying);
    }, [isPlaying, onPlayingChange]);

    useEffect(() => {
      if (error) console.warn('Recording playback failed:', error);
    }, [error]);

    // Nothing to play: a meeting saved without audio, or a folder since moved.
    if (lookedUp && !audioPath) return null;
    if (!audioPath) return null;

    const progress = duration > 0 ? Math.min(100, (currentTime / duration) * 100) : 0;

    return (
      <div className="border-t border-[var(--af-border)] bg-[var(--af-panel)] px-4 py-2.5 sm:px-6 lg:px-8">
        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={() => (isPlaying ? pause() : play())}
            aria-label={isPlaying ? t('playerPause') : t('playerPlay')}
            title={isPlaying ? t('playerPause') : t('playerPlay')}
            className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-[var(--af-accent)] text-[var(--af-accent-contrast)] transition hover:opacity-90"
          >
            {isPlaying ? <Pause size={16} /> : <Play size={16} className="ml-0.5" />}
          </button>

          <span className="shrink-0 text-xs tabular-nums text-[var(--af-text-2)]">
            {formatTime(currentTime)}
          </span>

          <label className="relative flex min-w-0 flex-1 items-center">
            <span className="sr-only">{t('playerPosition')}</span>
            <span
              aria-hidden
              className="pointer-events-none absolute inset-x-0 h-1 rounded-full bg-[var(--af-border)]"
            />
            <span
              aria-hidden
              className="pointer-events-none absolute h-1 rounded-full bg-[var(--af-accent)]"
              style={{ width: `${progress}%` }}
            />
            <input
              type="range"
              min={0}
              max={duration || 0}
              step={0.1}
              value={Math.min(currentTime, duration || 0)}
              onChange={(event) => seek(Number(event.target.value))}
              className="relative h-4 w-full cursor-pointer appearance-none bg-transparent
                         [&::-webkit-slider-thumb]:h-3 [&::-webkit-slider-thumb]:w-3
                         [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full
                         [&::-webkit-slider-thumb]:bg-[var(--af-accent)]"
            />
          </label>

          <span className="shrink-0 text-xs tabular-nums text-[var(--af-text-3)]">
            {formatTime(duration)}
          </span>
        </div>

        {/* The reason is technical and English; the reader gets the fact. */}
        {error && <p className="mt-1 text-xs text-red-400">{t('playerError')}</p>}
      </div>
    );
  },
);

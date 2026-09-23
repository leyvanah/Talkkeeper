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
import { useLocale, useTranslations } from 'next-intl';
import { Pause, Play } from 'lucide-react';
import { PLAYBACK_RATES, useAudioPlayer } from '@/hooks/useAudioPlayer';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { TranscriptSegmentData } from '@/types';
import { Playhead } from '@/lib/playhead';

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
  /** Where the position is published every frame, for views that draw it. */
  playhead?: Playhead;
  /** Called with whether this meeting has a recording to play at all. */
  onAvailabilityChange?: (available: boolean) => void;
}

/** `1×`, `1,5×` — with the reader's own decimal mark. */
function formatRate(rate: number, locale: string): string {
  return `${rate.toLocaleString(locale, { maximumFractionDigits: 2 })}×`;
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
    { meetingFolderPath, segments, onActiveSegmentChange, onPlayingChange, playhead, onAvailabilityChange },
    ref,
  ) {
    const t = useTranslations('meetingDetails');
    const [audioPath, setAudioPath] = useState<string | null>(null);
    const [lookedUp, setLookedUp] = useState(false);
    const locale = useLocale();
    const { isPlaying, currentTime, duration, error, play, pause, seek, rate, setRate } =
      useAudioPlayer(audioPath);

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

    useEffect(() => {
      onAvailabilityChange?.(!!audioPath);
    }, [audioPath, onAvailabilityChange]);

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

    // The exact position, for the views that draw a moving line. They move
    // their own element, so this does not re-render them.
    useEffect(() => {
      playhead?.update(currentTime, isPlaying);
    }, [playhead, currentTime, isPlaying]);

    useEffect(() => {
      if (error) console.warn('Recording playback failed:', error);
    }, [error]);

    // Nothing to play: a meeting saved without audio, or a folder since moved.
    if (lookedUp && !audioPath) return null;
    if (!audioPath) return null;

    const progress = duration > 0 ? Math.min(100, (currentTime / duration) * 100) : 0;

    return (
      // One quiet line under the transcript: no panel of its own, and the
      // accent left to the progress, not a filled button beside it.
      <div className="border-t border-[var(--af-border)] px-3 py-1.5 sm:px-5 lg:px-7">
        <div className="flex items-center gap-2.5">
          <button
            type="button"
            onClick={() => (isPlaying ? pause() : play())}
            aria-label={isPlaying ? t('playerPause') : t('playerPlay')}
            title={isPlaying ? t('playerPause') : t('playerPlay')}
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-[var(--af-text)] transition-colors hover:bg-[var(--af-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)]"
          >
            {isPlaying ? <Pause size={17} /> : <Play size={17} className="ml-0.5" />}
          </button>

          <span className="shrink-0 text-xs tabular-nums text-[var(--af-text-3)]">
            {formatTime(currentTime)}
          </span>

          <label className="relative flex min-w-0 flex-1 items-center">
            <span className="sr-only">{t('playerPosition')}</span>
            <span
              aria-hidden
              className="pointer-events-none absolute inset-x-0 h-[3px] rounded-full bg-[var(--af-border)]"
            />
            <span
              aria-hidden
              className="pointer-events-none absolute h-[3px] rounded-full bg-[var(--af-accent)]"
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
                         [&::-webkit-slider-thumb]:h-2.5 [&::-webkit-slider-thumb]:w-2.5
                         [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full
                         [&::-webkit-slider-thumb]:bg-[var(--af-accent)]"
            />
          </label>

          <span className="shrink-0 text-xs tabular-nums text-[var(--af-text-3)]">
            {formatTime(duration)}
          </span>

          <DropdownMenu>
            <DropdownMenuTrigger
              aria-label={t('playerSpeed')}
              title={t('playerSpeed')}
              // Marked only when it is not the usual speed.
              className={`min-w-[2.25rem] shrink-0 rounded-md px-1.5 py-1 text-xs tabular-nums transition-colors hover:bg-[var(--af-hover)] ${
                rate === 1
                  ? 'text-[var(--af-text-3)] hover:text-[var(--af-text)]'
                  : 'font-medium text-[var(--af-accent)]'
              }`}
            >
              {formatRate(rate, locale)}
            </DropdownMenuTrigger>
            <DropdownMenuContent side="top" align="end" className="min-w-[5rem]">
              <DropdownMenuRadioGroup
                value={String(rate)}
                onValueChange={(value) => setRate(Number(value))}
              >
                {[...PLAYBACK_RATES].reverse().map((option) => (
                  <DropdownMenuRadioItem key={option} value={String(option)} className="tabular-nums">
                    {formatRate(option, locale)}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>

        {/* The reason is technical and English, so it lives in the tooltip;
            the reader gets the fact, and whoever is asked to look gets the
            reason without opening a console. */}
        {error && (
          <p className="mt-1 text-xs text-red-400" title={error}>
            {t('playerError')}
          </p>
        )}
      </div>
    );
  },
);

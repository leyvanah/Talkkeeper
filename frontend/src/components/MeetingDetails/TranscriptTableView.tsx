"use client";

/**
 * The transcript along a time ruler: the client's lines on one side, the
 * host's on the other, every line at the moment it starts.
 *
 * The chat view reads well but loses what the separate tracks were recorded
 * for: when two people talk at once. Here the ruler runs down the left edge
 * with a mark for every second, and each line sits on the mark of its own
 * start, so an interruption appears beside the part of the other person's
 * sentence it cut into. Where there is more text than the seconds have room
 * for, the ruler stretches rather than the lines drifting off their marks —
 * see `lib/transcript-table`.
 *
 * Shares the chat view's contract with the player: the active line is named
 * by id, a click on a line seeks there, and the view follows the playhead only
 * while it plays and only when the line has scrolled out of sight.
 */

import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Minus, Plus } from 'lucide-react';
import { useTranslations } from 'next-intl';
import { TranscriptSegmentData } from '@/types';
import {
  cleanStopWords,
  displaySpeaker,
  mergeAdjacentSameSpeaker,
  speakerColor,
  speakerDot,
} from '@/components/VirtualizedTranscriptView';
import {
  layoutTimeline,
  PlacedLine,
  ticksBetween,
  timeAt,
  TimelineLine,
  yAt,
} from '@/lib/transcript-table';
import { getSpeakerSides, sideOf, SpeakerSide } from '@/services/speakerRoleService';

interface TranscriptTableViewProps {
  segments: TranscriptSegmentData[];
  meetingId?: string;
  /** Changes when the set of speakers may have changed (a rename, a rerun). */
  speakersKey?: string;
  onRenameSpeaker?: (speaker: string) => void;
  activeSegmentId?: string | null;
  onSeekTo?: (seconds: number) => void;
  followActiveSegment?: boolean;
  hasMore?: boolean;
  isLoadingMore?: boolean;
  totalCount?: number;
  loadedCount?: number;
  onLoadMore?: () => void;
}

/** Width of the ruler, in pixels. */
const RULER = 64;
/** Space between the ruler and a column, and between the two columns. */
const GUTTER = 12;
/** Space to the right of the second column, clear of the scrollbar. */
const EDGE = 16;
/** Height of a line's name-and-time row. */
const LABEL_ROW = 16;
/** Height of the sticky column header. */
const HEADER = 32;

/** Pixels per second of recording, from overview to tenths of a second. */
const SCALES = [16, 32, 64, 128] as const;
const DEFAULT_SCALE = 32;
const SCALE_STORAGE_KEY = 'transcript_timeline_scale';

const LAYOUT = { gap: 6, padding: 12 };

/** `m:ss`, `h:mm:ss` past an hour, with tenths when asked. */
function clock(seconds: number, tenths = false): string {
  const total = Math.max(0, seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  const secText = tenths
    ? (Math.floor(secs * 10) / 10).toFixed(1).padStart(4, '0')
    : String(Math.floor(secs)).padStart(2, '0');
  const mm = hours > 0 ? String(minutes).padStart(2, '0') : String(minutes);
  return hours > 0 ? `${hours}:${mm}:${secText}` : `${mm}:${secText}`;
}

/** A guess at a line's height until it has been measured. */
function estimateHeight(line: TimelineLine): number {
  return 46 + 20 * Math.max(0, Math.ceil(line.text.length / 42) - 1);
}

/** Horizontal placement of a column, as CSS. */
function columnBox(column: PlacedLine['column']): { left: string; width: string } {
  const both = `calc(100% - ${RULER + GUTTER + EDGE}px)`;
  const half = `calc((100% - ${RULER + GUTTER * 2 + EDGE}px) / 2)`;
  if (column === 'both') return { left: `${RULER + GUTTER}px`, width: both };
  if (column === 'client') return { left: `${RULER + GUTTER}px`, width: half };
  return {
    left: `calc(${RULER + GUTTER * 2}px + (100% - ${RULER + GUTTER * 2 + EDGE}px) / 2)`,
    width: half,
  };
}

const Bubble = memo(function Bubble({
  placed,
  userName,
  isActive,
  onSeekTo,
  onRenameSpeaker,
  onMeasure,
}: {
  placed: PlacedLine;
  userName: string;
  isActive: boolean;
  onSeekTo?: (seconds: number) => void;
  onRenameSpeaker?: (speaker: string) => void;
  onMeasure: (id: string, element: HTMLElement | null) => void;
}) {
  const t = useTranslations('recording');
  const { line, column } = placed;
  const host = column === 'host';
  const text = cleanStopWords(line.text) || line.text;
  const ref = useCallback((element: HTMLDivElement | null) => onMeasure(line.id, element), [
    line.id,
    onMeasure,
  ]);
  const seek = onSeekTo ? () => onSeekTo(line.start) : undefined;

  return (
    <div
      ref={ref}
      id={`table-line-${line.id}`}
      className="absolute"
      // The header line is centred on the line's position, so the start time
      // reads on the ruler mark it belongs to.
      style={{ top: placed.top - LABEL_ROW / 2, ...columnBox(column) }}
    >
      <div className="flex items-center gap-1.5 text-[11px]" style={{ height: LABEL_ROW }}>
        <span aria-hidden className={`h-1.5 w-1.5 shrink-0 rounded-full ${speakerDot(line.speaker)}`} />
        {line.speaker &&
          (onRenameSpeaker ? (
            <button
              type="button"
              onClick={() => onRenameSpeaker(line.speaker!)}
              title={t('renameSpeakerTitle', { speaker: line.speaker })}
              className={`truncate font-semibold ${speakerColor(line.speaker)} hover:underline`}
            >
              {displaySpeaker(line.speaker, userName)}
            </button>
          ) : (
            <span className={`truncate font-semibold ${speakerColor(line.speaker)}`}>
              {displaySpeaker(line.speaker, userName)}
            </span>
          ))}
        <span className="shrink-0 tabular-nums text-[var(--af-text-3)]">
          {clock(line.start, true)}
          {line.end != null && line.end > line.start && ` – ${clock(line.end, true)}`}
        </span>
      </div>
      <div
        role={seek ? 'button' : undefined}
        tabIndex={seek ? 0 : undefined}
        title={seek ? t('playFromHere') : undefined}
        onClick={seek}
        onKeyDown={
          seek
            ? (event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  seek();
                }
              }
            : undefined
        }
        className={[
          'mt-1 rounded-lg border px-2.5 py-1.5 text-sm leading-relaxed',
          host || column === 'both'
            ? 'border-blue-500/25 bg-blue-500/10 text-[var(--af-text)]'
            : 'border-[var(--af-border)] bg-[var(--af-panel-2)] text-[var(--af-text-2)]',
          seek ? 'cursor-pointer transition-colors' : '',
          isActive ? 'ring-2 ring-[var(--af-accent)]' : '',
        ]
          .filter(Boolean)
          .join(' ')}
      >
        {text}
      </div>
    </div>
  );
});

export function TranscriptTableView({
  segments,
  meetingId,
  speakersKey = '',
  onRenameSpeaker,
  activeSegmentId = null,
  onSeekTo,
  followActiveSegment = false,
  hasMore = false,
  isLoadingMore = false,
  totalCount = 0,
  loadedCount = 0,
  onLoadMore,
}: TranscriptTableViewProps) {
  const t = useTranslations('meetingDetails');
  const tr = useTranslations('recording');

  const [userName, setUserName] = useState('');
  const [scale, setScale] = useState<number>(DEFAULT_SCALE);
  useEffect(() => {
    try {
      setUserName(localStorage.getItem('meetily_user_name')?.trim() || '');
      const saved = Number(localStorage.getItem(SCALE_STORAGE_KEY));
      if ((SCALES as readonly number[]).includes(saved)) setScale(saved);
    } catch {
      // Defaults stand.
    }
  }, []);

  // Which side each speaker is on. Until the backend answers, labels decide,
  // which is the right answer for every call anyway.
  const [sides, setSides] = useState<SpeakerSide[]>([]);
  useEffect(() => {
    if (!meetingId) {
      setSides([]);
      return;
    }
    let cancelled = false;
    getSpeakerSides(meetingId)
      .then((next) => {
        if (!cancelled) setSides(next);
      })
      .catch((error) => console.warn('Could not read speaker roles:', error));
    return () => {
      cancelled = true;
    };
  }, [meetingId, speakersKey]);

  // The same turns the chat view and the player use, so ids match.
  const lines = useMemo<TimelineLine[]>(
    () =>
      mergeAdjacentSameSpeaker(segments).map((segment) => ({
        id: segment.id,
        start: segment.timestamp ?? 0,
        end: segment.endTime,
        text: segment.text,
        speaker: segment.speaker,
        confidence: segment.confidence,
      })),
    [segments],
  );

  // Heights are measured after the first paint; a change re-runs the layout
  // once per frame at most.
  const heights = useRef(new Map<string, number>());
  const observed = useRef(new Map<string, HTMLElement>());
  const [measureVersion, setMeasureVersion] = useState(0);
  const frame = useRef<number | null>(null);
  const bump = useCallback(() => {
    if (frame.current != null) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = null;
      setMeasureVersion((v) => v + 1);
    });
  }, []);
  const resizeObserver = useMemo(
    () =>
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver((entries) => {
            let changed = false;
            for (const entry of entries) {
              const id = (entry.target as HTMLElement).dataset.lineId;
              if (!id) continue;
              const height = Math.ceil((entry.target as HTMLElement).offsetHeight);
              if (heights.current.get(id) !== height) {
                heights.current.set(id, height);
                changed = true;
              }
            }
            if (changed) bump();
          }),
    [bump],
  );
  useEffect(() => () => resizeObserver?.disconnect(), [resizeObserver]);
  const onMeasure = useCallback(
    (id: string, element: HTMLElement | null) => {
      const previous = observed.current.get(id);
      if (previous && previous !== element) {
        resizeObserver?.unobserve(previous);
        observed.current.delete(id);
      }
      if (element && previous !== element) {
        element.dataset.lineId = id;
        observed.current.set(id, element);
        resizeObserver?.observe(element);
      }
    },
    [resizeObserver],
  );

  const layout = useMemo(
    () =>
      layoutTimeline(
        lines,
        (speaker) => sideOf(sides, speaker),
        (line) => heights.current.get(line.id) ?? estimateHeight(line),
        { pxPerSecond: scale, ...LAYOUT },
      ),
    // measureVersion stands for the heights, which live in a ref.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [lines, sides, scale, measureVersion],
  );

  // The visible stretch, for drawing only the ruler marks that can be seen.
  const scrollRef = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 800 });
  const readView = useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;
    setView((previous) =>
      previous.top === element.scrollTop && previous.height === element.clientHeight
        ? previous
        : { top: element.scrollTop, height: element.clientHeight },
    );
  }, []);
  useEffect(() => {
    readView();
    const element = scrollRef.current;
    if (!element || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(readView);
    observer.observe(element);
    return () => observer.disconnect();
  }, [readView]);

  const ticks = useMemo(() => {
    const from = view.top - HEADER - 200;
    const to = view.top - HEADER + view.height + 200;
    return ticksBetween(layout, Math.max(0, from), Math.min(layout.height, to));
  }, [layout, view]);

  // Changing the scale keeps the moment at the top of the view where it was.
  const pinnedTime = useRef<number | null>(null);
  const changeScale = useCallback(
    (direction: 1 | -1) => {
      const index = SCALES.indexOf(scale as (typeof SCALES)[number]);
      const next = SCALES[Math.min(SCALES.length - 1, Math.max(0, index + direction))];
      if (next === scale) return;
      const element = scrollRef.current;
      if (element) pinnedTime.current = timeAt(layout, element.scrollTop);
      setScale(next);
      try {
        localStorage.setItem(SCALE_STORAGE_KEY, String(next));
      } catch {
        // Not remembered this time.
      }
    },
    [layout, scale],
  );
  useLayoutEffect(() => {
    if (pinnedTime.current == null) return;
    const element = scrollRef.current;
    if (element) element.scrollTop = yAt(layout, pinnedTime.current);
    pinnedTime.current = null;
  }, [layout]);

  // Follow the playhead, the way the chat view does.
  useEffect(() => {
    if (!followActiveSegment || !activeSegmentId) return;
    const placed = layout.placed.find((entry) => entry.line.id === activeSegmentId);
    const element = scrollRef.current;
    if (!placed || !element) return;
    const top = placed.top + HEADER;
    const bottom = top + placed.height;
    const visibleTop = element.scrollTop + HEADER;
    const visibleBottom = element.scrollTop + element.clientHeight;
    if (top >= visibleTop && bottom <= visibleBottom) return;
    element.scrollTo({ top: top - element.clientHeight / 3, behavior: 'smooth' });
  }, [activeSegmentId, followActiveSegment, layout]);

  // Load the next page when the end comes into view.
  const onScroll = useCallback(() => {
    readView();
    const element = scrollRef.current;
    if (!element || !onLoadMore || !hasMore || isLoadingMore) return;
    if (element.scrollHeight - element.scrollTop - element.clientHeight < 200) {
      onLoadMore();
    }
  }, [readView, onLoadMore, hasMore, isLoadingMore]);

  const scaleIndex = SCALES.indexOf(scale as (typeof SCALES)[number]);

  return (
    <div ref={scrollRef} onScroll={onScroll} className="relative h-full min-h-0 overflow-y-auto">
      {/* Header, inside the scroller so it lines up with the columns whatever
          the scrollbar takes. */}
      <div
        className="sticky top-0 z-20 grid items-center border-b border-[var(--af-border)] bg-[var(--af-bg)] text-[11px] font-semibold uppercase tracking-wide text-[var(--af-text-3)]"
        style={{
          height: HEADER,
          gridTemplateColumns: `${RULER}px 1fr 1fr`,
          columnGap: GUTTER,
          paddingRight: EDGE,
        }}
      >
        <div className="flex h-full items-center justify-center gap-0.5 border-r border-[var(--af-border)]">
          <button
            type="button"
            onClick={() => changeScale(-1)}
            disabled={scaleIndex <= 0}
            title={t('timelineZoomOut')}
            aria-label={t('timelineZoomOut')}
            className="rounded p-0.5 hover:text-[var(--af-text)] disabled:opacity-30"
          >
            <Minus size={13} />
          </button>
          <button
            type="button"
            onClick={() => changeScale(1)}
            disabled={scaleIndex >= SCALES.length - 1}
            title={t('timelineZoomIn')}
            aria-label={t('timelineZoomIn')}
            className="rounded p-0.5 hover:text-[var(--af-text)] disabled:opacity-30"
          >
            <Plus size={13} />
          </button>
        </div>
        <div>{t('tableColumnClient')}</div>
        <div>{userName || t('tableColumnHost')}</div>
      </div>

      {lines.length === 0 ? (
        <p className="mt-8 text-center text-sm text-[var(--af-text-3)]">
          {tr('startRecordingToSeeTranscription')}
        </p>
      ) : (
        <div className="relative w-full" style={{ height: layout.height }}>
          {/* The ruler: flush with the left edge of the panel. */}
          <div
            aria-hidden
            className="absolute left-0 top-0 h-full border-r border-[var(--af-border)]"
            style={{ width: RULER }}
          >
            {ticks.map((tick) => (
              <div
                key={tick.t}
                className="absolute right-0 flex items-center"
                style={{ top: tick.y, transform: 'translateY(-50%)' }}
              >
                {tick.kind === 'label' && (
                  <span className="mr-1 text-[10px] tabular-nums text-[var(--af-text-3)]">
                    {clock(tick.t)}
                  </span>
                )}
                <span
                  className="block h-px bg-[var(--af-text-3)]"
                  style={{
                    width: tick.kind === 'label' ? 10 : tick.kind === 'major' ? 6 : 3,
                    opacity: tick.kind === 'minor' ? 0.5 : 0.9,
                  }}
                />
              </div>
            ))}
            {/* Where each line starts, on the ruler itself. */}
            {layout.placed.map((placed) => (
              <span
                key={`mark-${placed.line.id}`}
                className={`absolute h-2 w-2 rounded-full ring-2 ring-[var(--af-bg)] ${
                  placed.column === 'client' ? 'bg-purple-500' : 'bg-blue-500'
                }`}
                style={{ top: placed.top, right: -4, transform: 'translateY(-50%)' }}
              />
            ))}
          </div>

          {/* Faint guides across the columns at labelled marks. */}
          {ticks
            .filter((tick) => tick.kind === 'label')
            .map((tick) => (
              <div
                key={`guide-${tick.t}`}
                aria-hidden
                className="pointer-events-none absolute h-px bg-[var(--af-border)] opacity-40"
                style={{ top: tick.y, left: RULER, right: 0 }}
              />
            ))}

          {/* How long each line lasted, down its left edge on the same ruler:
              what makes an interjection visibly land inside a longer line. */}
          {layout.placed.map(({ line, column, top }) =>
            line.end != null && line.end > line.start ? (
              <div
                key={`span-${line.id}`}
                aria-hidden
                className={`pointer-events-none absolute w-0.5 rounded-full ${
                  column === 'client' ? 'bg-purple-500/50' : 'bg-blue-500/50'
                }`}
                style={{
                  top,
                  height: Math.max(2, yAt(layout, line.end) - top),
                  left: `calc(${columnBox(column).left} - 6px)`,
                }}
              />
            ) : null,
          )}

          {layout.placed.map((placed) => (
            <Bubble
              key={placed.line.id}
              placed={placed}
              userName={userName}
              isActive={placed.line.id === activeSegmentId}
              onSeekTo={onSeekTo}
              onRenameSpeaker={onRenameSpeaker}
              onMeasure={onMeasure}
            />
          ))}
        </div>
      )}

      {(hasMore || isLoadingMore) && lines.length > 0 && (
        <div className="flex items-center justify-center py-4 text-sm text-[var(--af-text-3)]">
          {isLoadingMore
            ? tr('loadingMore')
            : totalCount > 0
              ? tr('showingSegments', { loaded: loadedCount, total: totalCount })
              : null}
        </div>
      )}
    </div>
  );
}

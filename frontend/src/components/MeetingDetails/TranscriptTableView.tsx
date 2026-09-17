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
 * A line's box runs the whole length of its time, and its text is spread down
 * the box — each sentence near the moment it was said, on the assumption of an
 * even pace — so the moving playhead passes a sentence while it is being
 * spoken, and a long answer does not leave its last half minute blank.
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
  piecesFromWords,
  placeAt,
  placePieces,
  splitForTimeline,
  TimedWord,
  ticksBetween,
  timeAt,
  TimelineLayout,
  TimelineLine,
  wordsAt,
  yAt,
} from '@/lib/transcript-table';
import { getSpeakerSides, sideOf, SpeakerSide } from '@/services/speakerRoleService';
import { Playhead } from '@/lib/playhead';

interface TranscriptTableViewProps {
  segments: TranscriptSegmentData[];
  meetingId?: string;
  /** Changes when the set of speakers may have changed (a rename, a rerun). */
  speakersKey?: string;
  onRenameSpeaker?: (speaker: string) => void;
  activeSegmentId?: string | null;
  onSeekTo?: (seconds: number) => void;
  followActiveSegment?: boolean;
  /** The player's position, drawn as a pointer moving down the ruler. */
  playhead?: Playhead;
  hasMore?: boolean;
  isLoadingMore?: boolean;
  totalCount?: number;
  loadedCount?: number;
  onLoadMore?: () => void;
}

/** Width of the ruler: just the labels, and room for `h:mm:ss` past an hour. */
const RULER = 32;
const RULER_LONG = 40;
/** Space between the ruler and a column, and between the two columns. */
const GUTTER = 10;
/** Space to the right of the second column, clear of the scrollbar. */
const EDGE = 16;
/** Height of a line's name-and-time row, and the space under it. */
const LABEL_ROW = 16;
const LABEL_GAP = 4;
/** The text box: border and inner padding, and the space between pieces. */
const BORDER = 1;
const PAD_Y = 6;
const PAD_X = 10;
const PIECE_GAP = 2;
/** Height of the sticky column header. */
const HEADER = 32;
/** How far the playhead pointer's tip reaches past the ruler. */
const POINTER_TIP = 7;
/** From a line's position on the ruler down to where its text begins. */
const TEXT_ORIGIN = LABEL_ROW / 2 + LABEL_GAP + BORDER + PAD_Y;

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

/** Key under which one piece of a line's text is measured. */
function pieceKey(lineId: string, index: number): string {
  return `${lineId}#${index}`;
}

/** A guess at a piece's height until it has been measured. */
function estimatePiece(piece: string): number {
  return 22 * Math.max(1, Math.ceil(piece.length / 42));
}

/** Horizontal placement of a column, as CSS. */
function columnBox(column: PlacedLine['column'], ruler: number): { left: string; width: string } {
  const both = `calc(100% - ${ruler + GUTTER + EDGE}px)`;
  const half = `calc((100% - ${ruler + GUTTER * 2 + EDGE}px) / 2)`;
  if (column === 'both') return { left: `${ruler + GUTTER}px`, width: both };
  if (column === 'client') return { left: `${ruler + GUTTER}px`, width: half };
  return {
    left: `calc(${ruler + GUTTER * 2}px + (100% - ${ruler + GUTTER * 2 + EDGE}px) / 2)`,
    width: half,
  };
}

/** Where a line's pieces go and how tall its box is. */
interface LineGeometry {
  pieceTops: number[];
  boxHeight: number;
}

/**
 * A piece of a line's text. With word timings it knows when it began and which
 * words it holds; without, it is placed by an even-pace estimate.
 */
interface Piece {
  text: string;
  start?: number;
  words?: TimedWord[];
  /** Index of the piece's first word within the line. */
  firstWord?: number;
}

function piecesForLine(line: TimelineLine): Piece[] {
  if (line.words?.length) {
    let first = 0;
    return piecesFromWords(line.words).map((piece) => {
      const entry = { ...piece, firstWord: first };
      first += piece.words.length;
      return entry;
    });
  }
  return splitForTimeline(line.text).map((text) => ({ text }));
}

function wordId(lineId: string, index: number): string {
  return `table-word-${lineId}-${index}`;
}

/** The mark on the word being heard: a soft wash of the accent. */
function markWord(element: HTMLElement, on: boolean) {
  const wash = 'color-mix(in srgb, var(--af-accent) 24%, transparent)';
  element.style.backgroundColor = on ? wash : '';
  element.style.boxShadow = on ? `0 0 0 2px ${wash}` : '';
  element.style.borderRadius = on ? '3px' : '';
}

const Bubble = memo(function Bubble({
  placed,
  pieces,
  geometry,
  ruler,
  userName,
  isActive,
  onSeekTo,
  onRenameSpeaker,
  onMeasure,
}: {
  placed: PlacedLine;
  pieces: Piece[];
  geometry: LineGeometry;
  ruler: number;
  userName: string;
  isActive: boolean;
  onSeekTo?: (seconds: number) => void;
  onRenameSpeaker?: (speaker: string) => void;
  onMeasure: (key: string, element: HTMLElement | null) => void;
}) {
  const t = useTranslations('recording');
  const { line, column } = placed;
  const host = column !== 'client';
  const seek = onSeekTo ? () => onSeekTo(line.start) : undefined;

  return (
    <div
      id={`table-line-${line.id}`}
      className="absolute"
      // The name-and-time row is centred on the line's position, so the start
      // reads on the ruler mark it belongs to.
      style={{ top: placed.top - LABEL_ROW / 2, ...columnBox(column, ruler) }}
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
          'relative rounded-lg border text-sm leading-relaxed',
          host
            ? 'border-blue-500/25 bg-blue-500/10 text-[var(--af-text)]'
            : 'border-[var(--af-border)] bg-[var(--af-panel-2)] text-[var(--af-text-2)]',
          seek ? 'cursor-pointer transition-colors' : '',
          isActive ? 'ring-2 ring-[var(--af-accent)]' : '',
        ]
          .filter(Boolean)
          .join(' ')}
        style={{ marginTop: LABEL_GAP, height: geometry.boxHeight, borderWidth: BORDER }}
      >
        {pieces.map((piece, index) => (
          <p
            key={index}
            ref={(element) => onMeasure(pieceKey(line.id, index), element)}
            className="absolute"
            style={{ top: PAD_Y + (geometry.pieceTops[index] ?? 0), left: PAD_X, right: PAD_X }}
          >
            {piece.words
              ? piece.words.map((word, offset) => (
                  <span key={offset}>
                    {offset > 0 && ' '}
                    <span id={wordId(line.id, (piece.firstWord ?? 0) + offset)}>{word.w}</span>
                  </span>
                ))
              : piece.text}
          </p>
        ))}
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
  playhead,
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
        // A timed line shows exactly the words that were timed; the filler
        // cleanup would leave words with no place in the text.
        text: segment.words?.length
          ? segment.words.map((word) => word.w).join(' ')
          : cleanStopWords(segment.text) || segment.text,
        speaker: segment.speaker,
        confidence: segment.confidence,
        words: segment.words,
      })),
    [segments],
  );
  const piecesOf = useMemo(
    () => new Map(lines.map((line) => [line.id, piecesForLine(line)])),
    [lines],
  );

  // Pieces are measured after the first paint; a change re-runs the layout
  // once per frame at most. Their width is fixed by the column, so their
  // height never depends on where they end up.
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
              const key = (entry.target as HTMLElement).dataset.pieceKey;
              if (!key) continue;
              const height = Math.ceil((entry.target as HTMLElement).offsetHeight);
              if (heights.current.get(key) !== height) {
                heights.current.set(key, height);
                changed = true;
              }
            }
            if (changed) bump();
          }),
    [bump],
  );
  useEffect(() => () => resizeObserver?.disconnect(), [resizeObserver]);
  const onMeasure = useCallback(
    (key: string, element: HTMLElement | null) => {
      const previous = observed.current.get(key);
      if (previous === element) return;
      if (previous) {
        resizeObserver?.unobserve(previous);
        observed.current.delete(key);
      }
      if (element) {
        element.dataset.pieceKey = key;
        observed.current.set(key, element);
        resizeObserver?.observe(element);
      }
    },
    [resizeObserver],
  );
  const pieceHeights = useCallback(
    (line: TimelineLine) =>
      (piecesOf.get(line.id) ?? []).map(
        (piece, index) =>
          heights.current.get(pieceKey(line.id, index)) ?? estimatePiece(piece.text),
      ),
    [piecesOf],
  );

  // The room a line needs with its text packed tight; the layout reserves
  // this, and any extra length a line has in time is filled by spreading.
  const layout = useMemo(
    () =>
      layoutTimeline(
        lines,
        (speaker) => sideOf(sides, speaker),
        (line) => {
          const own = pieceHeights(line);
          const text = own.reduce((sum, h) => sum + h, 0) + PIECE_GAP * Math.max(0, own.length - 1);
          return LABEL_ROW / 2 + LABEL_GAP + BORDER * 2 + PAD_Y * 2 + text;
        },
        { pxPerSecond: scale, ...LAYOUT },
      ),
    // measureVersion stands for the heights, which live in a ref.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [lines, sides, scale, measureVersion, pieceHeights],
  );

  const ruler = useMemo(
    () => (layout.anchors[layout.anchors.length - 1].t >= 3600 ? RULER_LONG : RULER),
    [layout],
  );

  // Each line's box and the places of its pieces.
  const geometry = useMemo(
    () => spreadLines(layout, piecesOf, pieceHeights),
    // measureVersion: see above.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [layout, piecesOf, pieceHeights, measureVersion],
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

  // The playhead pointer. It is positioned straight on its element every frame;
  // re-rendering the table sixty times a second is what the playhead store
  // exists to avoid.
  // Every timed word of the meeting, in order, for marking the one being heard.
  const timedWords = useMemo(() => {
    const all: Array<TimedWord & { id: string }> = [];
    for (const line of lines) {
      line.words?.forEach((word, index) => all.push({ ...word, id: wordId(line.id, index) }));
    }
    return all.sort((a, b) => a.s - b.s);
  }, [lines]);
  const timedWordsRef = useRef(timedWords);
  timedWordsRef.current = timedWords;
  const markedWords = useRef<string[]>([]);

  const lineRef = useRef<HTMLDivElement>(null);
  const lineLabelRef = useRef<HTMLSpanElement>(null);
  const layoutRef = useRef(layout);
  layoutRef.current = layout;
  const drawPlayhead = useCallback((seconds: number, playing: boolean, follow: boolean) => {
    const line = lineRef.current;
    const element = scrollRef.current;
    if (!line || !element) return;
    const y = yAt(layoutRef.current, seconds);
    line.style.transform = `translateY(${y}px)`;
    line.style.visibility = playing || seconds > 0 ? 'visible' : 'hidden';
    if (lineLabelRef.current) lineLabelRef.current.textContent = clock(seconds, true);

    // The words being heard. Only a change touches the page.
    const words = timedWordsRef.current;
    const now = playing || seconds > 0 ? wordsAt(words, seconds).map((index) => words[index].id) : [];
    const before = markedWords.current;
    if (now.length !== before.length || now.some((id, index) => id !== before[index])) {
      for (const id of before) {
        if (!now.includes(id)) {
          const element = document.getElementById(id);
          if (element) markWord(element, false);
        }
      }
      for (const id of now) {
        const element = document.getElementById(id);
        if (element) markWord(element, true);
      }
      markedWords.current = now;
    }

    if (!follow || !playing) return;
    // Keep it in view while playing: when it leaves the lower part of the
    // screen, bring it back to the upper third in one step.
    const onScreen = y + HEADER - element.scrollTop;
    if (onScreen < HEADER || onScreen > element.clientHeight * 0.85) {
      element.scrollTop = y + HEADER - element.clientHeight / 3;
    }
  }, []);
  const followRef = useRef(followActiveSegment);
  followRef.current = followActiveSegment;
  useEffect(() => {
    if (!playhead) return;
    return playhead.subscribe((seconds, playing) => drawPlayhead(seconds, playing, followRef.current));
  }, [playhead, drawPlayhead]);
  // A new layout (zoom, measured heights) moves the line without the player
  // having moved.
  useLayoutEffect(() => {
    if (playhead) drawPlayhead(playhead.time(), playhead.playing(), false);
  }, [layout, playhead, drawPlayhead]);

  // Without a playhead, follow the spoken line, the way the chat view does.
  useEffect(() => {
    if (playhead || !followActiveSegment || !activeSegmentId) return;
    const placed = layout.placed.find((entry) => entry.line.id === activeSegmentId);
    const element = scrollRef.current;
    if (!placed || !element) return;
    const top = placed.top + HEADER;
    const bottom = top + placed.height;
    const visibleTop = element.scrollTop + HEADER;
    const visibleBottom = element.scrollTop + element.clientHeight;
    if (top >= visibleTop && bottom <= visibleBottom) return;
    element.scrollTo({ top: top - element.clientHeight / 3, behavior: 'smooth' });
  }, [activeSegmentId, followActiveSegment, layout, playhead]);

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
          gridTemplateColumns: `${ruler}px 1fr 1fr`,
          columnGap: GUTTER,
          paddingRight: EDGE,
        }}
      >
        <div className="flex h-full flex-col items-center justify-center border-r border-[var(--af-border)]">
          <button
            type="button"
            onClick={() => changeScale(1)}
            disabled={scaleIndex >= SCALES.length - 1}
            title={t('timelineZoomIn')}
            aria-label={t('timelineZoomIn')}
            className="leading-none hover:text-[var(--af-text)] disabled:opacity-30"
          >
            <Plus size={12} />
          </button>
          <button
            type="button"
            onClick={() => changeScale(-1)}
            disabled={scaleIndex <= 0}
            title={t('timelineZoomOut')}
            aria-label={t('timelineZoomOut')}
            className="leading-none hover:text-[var(--af-text)] disabled:opacity-30"
          >
            <Minus size={12} />
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
            className={`absolute left-0 top-0 h-full border-r border-[var(--af-border)] ${
              onSeekTo ? 'cursor-pointer' : ''
            }`}
            style={{ width: ruler }}
            title={onSeekTo ? tr('playFromHere') : undefined}
            // A click on the ruler plays from that moment.
            onClick={
              onSeekTo
                ? (event) => {
                    const box = event.currentTarget.getBoundingClientRect();
                    onSeekTo(timeAt(layout, event.clientY - box.top));
                  }
                : undefined
            }
          >
            {ticks.map((tick) => (
              <div
                key={tick.t}
                className="absolute right-0 flex items-center"
                style={{ top: tick.y, transform: 'translateY(-50%)' }}
              >
                {tick.kind === 'label' && (
                  <span className="mr-0.5 text-[9px] tabular-nums leading-none text-[var(--af-text-3)]">
                    {clock(tick.t)}
                  </span>
                )}
                <span
                  className="block h-px bg-[var(--af-text-3)]"
                  style={{
                    width: tick.kind === 'minor' ? 2 : 4,
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
                style={{ top: tick.y, left: ruler, right: 0 }}
              />
            ))}

          {playhead && (
            <div
              ref={lineRef}
              aria-hidden
              className="pointer-events-none absolute left-0 top-0 z-10"
              style={{ visibility: 'hidden', willChange: 'transform' }}
            >
              {/* A pointer on the ruler, its tip on the moment being heard. The
                  accent can be white or black depending on the theme, so the
                  time takes the page colour to stay readable on it. */}
              <span
                ref={lineLabelRef}
                className="block -translate-y-1/2 whitespace-nowrap bg-[var(--af-accent)] pl-0.5 text-[9px] font-semibold tabular-nums leading-[14px] text-[var(--af-bg)]"
                style={{
                  width: ruler + POINTER_TIP,
                  paddingRight: POINTER_TIP,
                  clipPath: `polygon(0 0, calc(100% - ${POINTER_TIP}px) 0, 100% 50%, calc(100% - ${POINTER_TIP}px) 100%, 0 100%)`,
                }}
              />
            </div>
          )}

          {layout.placed.map((placed) => (
            <Bubble
              key={placed.line.id}
              placed={placed}
              pieces={piecesOf.get(placed.line.id) ?? [{ text: placed.line.text }]}
              geometry={geometry.get(placed.line.id) ?? { pieceTops: [], boxHeight: 0 }}
              ruler={ruler}
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

/**
 * Stretch each line's box over its time and spread its text down it.
 *
 * A box never reaches into the next line of its own column: two lines of one
 * side that overlap in time would otherwise cover each other, so the spread
 * is limited to the room before the next one.
 */
function spreadLines(
  layout: TimelineLayout,
  piecesOf: Map<string, Piece[]>,
  pieceHeights: (line: TimelineLine) => number[],
): Map<string, LineGeometry> {
  // The next top in each column, walking up from the bottom.
  const byTop = [...layout.placed].sort((a, b) => a.top - b.top);
  const nextTop = new Map<string, number>();
  const below = { host: Infinity, client: Infinity };
  for (let i = byTop.length - 1; i >= 0; i--) {
    const entry = byTop[i];
    const columns = entry.column === 'both' ? (['host', 'client'] as const) : [entry.column];
    nextTop.set(entry.line.id, Math.min(...columns.map((c) => below[c])));
    for (const c of columns) below[c] = entry.top;
  }

  const result = new Map<string, LineGeometry>();
  for (const entry of layout.placed) {
    const { line, top } = entry;
    const pieces = piecesOf.get(line.id) ?? [{ text: line.text }];
    const heights = pieceHeights(line);
    const packed =
      heights.reduce((sum, h) => sum + h, 0) + PIECE_GAP * Math.max(0, heights.length - 1);

    // How far the text may spread: to the end of the line's time, but not
    // into the next line of the same column.
    const end = Math.max(line.start, line.end ?? line.start);
    const spanEnd = yAt(layout, end);
    const limit = Math.min(spanEnd, nextTop.get(line.id)! - LABEL_ROW / 2 - LAYOUT.gap);
    // The text area ends here, measured from where the text begins.
    const textRoom = Math.max(packed, limit - top - TEXT_ORIGIN - PAD_Y - BORDER);

    // Where the text of a moment goes, measured from where the text begins.
    const offsetOf = (seconds: number) => Math.max(0, yAt(layout, seconds) - top - TEXT_ORIGIN);
    let pieceTops: number[];
    if (pieces.every((piece) => piece.start != null)) {
      // Timed: each piece at the moment its first word was said.
      pieceTops = placeAt(
        pieces.map((piece) => offsetOf(piece.start!)),
        heights,
        PIECE_GAP,
        textRoom,
      );
    } else {
      // Untimed: at the share of the line's time its text takes.
      const duration = end - line.start;
      const offsetAt = (fraction: number) =>
        duration <= 0 ? 0 : offsetOf(line.start + fraction * duration);
      pieceTops = placePieces(
        pieces.map((piece) => piece.text),
        heights,
        offsetAt,
        PIECE_GAP,
        textRoom,
      );
    }
    const lastBottom = pieceTops.length
      ? pieceTops[pieceTops.length - 1] + heights[heights.length - 1]
      : 0;
    const textBottom = Math.max(lastBottom, packed);
    const stretched = limit - (top - LABEL_ROW / 2 + LABEL_ROW + LABEL_GAP);
    result.set(line.id, {
      pieceTops,
      boxHeight: Math.max(textBottom + PAD_Y * 2 + BORDER * 2, stretched),
    });
  }
  return result;
}

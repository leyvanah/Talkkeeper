"use client";

/**
 * The transcript as a table: time, the client's lines, the host's lines.
 *
 * The chat view reads well but loses one thing the recording has: when two
 * people talk at once. Both sides are recorded on their own tracks with one
 * clock, so an interruption is two lines whose times overlap — here they sit
 * in the same row, side by side. Rows are built by `lib/transcript-table`;
 * which column a speaker goes in comes from the backend's speaker roles.
 *
 * Shares the chat view's contract with the player: the active line is named
 * by id, a click on a line seeks there, and the view follows the playhead only
 * while it plays and only when the line has scrolled out of sight.
 */

import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useTranslations } from 'next-intl';
import { TranscriptSegmentData } from '@/types';
import {
  cleanStopWords,
  displaySpeaker,
  formatRecordingTime,
  mergeAdjacentSameSpeaker,
  speakerColor,
  speakerDot,
} from '@/components/VirtualizedTranscriptView';
import { buildTableRows, rowIndexOf, TableLine, TableRow } from '@/lib/transcript-table';
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

/** Column layout shared by the header and every row. */
const GRID = 'grid grid-cols-[4.5rem_minmax(0,1fr)_minmax(0,1fr)] gap-x-3';

const Cell = memo(function Cell({
  lines,
  userName,
  activeSegmentId,
  onSeekTo,
  onRenameSpeaker,
  host,
}: {
  lines: TableLine[];
  userName: string;
  activeSegmentId: string | null;
  onSeekTo?: (seconds: number) => void;
  onRenameSpeaker?: (speaker: string) => void;
  host: boolean;
}) {
  const t = useTranslations('recording');
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      {lines.map((line, index) => {
        const isActive = line.id === activeSegmentId;
        const text = cleanStopWords(line.text) || line.text;
        // A speaker is named once per run, not on every line of a cell.
        const showSpeaker = !!line.speaker && lines[index - 1]?.speaker !== line.speaker;
        return (
          <div key={line.id} id={`table-line-${line.id}`} className="min-w-0">
            {showSpeaker && line.speaker && (
              <div className="mb-0.5 flex items-center gap-1.5">
                <span aria-hidden className={`h-1.5 w-1.5 shrink-0 rounded-full ${speakerDot(line.speaker)}`} />
                {onRenameSpeaker ? (
                  <button
                    type="button"
                    onClick={() => onRenameSpeaker(line.speaker!)}
                    title={t('renameSpeakerTitle', { speaker: line.speaker })}
                    className={`truncate text-[11px] font-semibold ${speakerColor(line.speaker)} hover:underline`}
                  >
                    {displaySpeaker(line.speaker, userName)}
                  </button>
                ) : (
                  <span className={`truncate text-[11px] font-semibold ${speakerColor(line.speaker)}`}>
                    {displaySpeaker(line.speaker, userName)}
                  </span>
                )}
              </div>
            )}
            <div
              role={onSeekTo ? 'button' : undefined}
              tabIndex={onSeekTo ? 0 : undefined}
              title={onSeekTo ? `${t('playFromHere')} · ${formatRecordingTime(line.start)}` : undefined}
              onClick={onSeekTo ? () => onSeekTo(line.start) : undefined}
              onKeyDown={
                onSeekTo
                  ? (event) => {
                      if (event.key === 'Enter' || event.key === ' ') {
                        event.preventDefault();
                        onSeekTo(line.start);
                      }
                    }
                  : undefined
              }
              className={[
                'rounded-lg border px-2.5 py-1.5 text-sm leading-relaxed',
                host
                  ? 'border-blue-500/25 bg-blue-500/10 text-[var(--af-text)]'
                  : 'border-[var(--af-border)] bg-[var(--af-panel-2)] text-[var(--af-text-2)]',
                onSeekTo ? 'cursor-pointer transition-colors' : '',
                isActive ? 'ring-2 ring-[var(--af-accent)]' : '',
              ]
                .filter(Boolean)
                .join(' ')}
            >
              {text}
            </div>
          </div>
        );
      })}
    </div>
  );
});

const Row = memo(function Row({
  row,
  userName,
  activeSegmentId,
  onSeekTo,
  onRenameSpeaker,
}: {
  row: TableRow;
  userName: string;
  activeSegmentId: string | null;
  onSeekTo?: (seconds: number) => void;
  onRenameSpeaker?: (speaker: string) => void;
}) {
  const overlap = row.host.length > 0 && row.client.length > 0;
  return (
    <div
      className={`${GRID} border-b border-[var(--af-border)] py-2.5 pl-2 ${
        // Two people at once: marked at the edge so it reads while scrolling.
        overlap ? 'shadow-[inset_2px_0_0_var(--af-accent)]' : ''
      }`}
    >
      <div className="pt-0.5 text-[11px] tabular-nums text-[var(--af-text-3)]">
        <div>{formatRecordingTime(row.start)}</div>
        {row.end - row.start >= 1 && (
          <div className="opacity-70">{formatRecordingTime(row.end)}</div>
        )}
      </div>
      <Cell
        lines={row.client}
        userName={userName}
        activeSegmentId={activeSegmentId}
        onSeekTo={onSeekTo}
        onRenameSpeaker={onRenameSpeaker}
        host={false}
      />
      <Cell
        lines={row.host}
        userName={userName}
        activeSegmentId={activeSegmentId}
        onSeekTo={onSeekTo}
        onRenameSpeaker={onRenameSpeaker}
        host
      />
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
  useEffect(() => {
    if (typeof window !== 'undefined') {
      setUserName(localStorage.getItem('meetily_user_name')?.trim() || '');
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
  const rows = useMemo(() => {
    const lines: TableLine[] = mergeAdjacentSameSpeaker(segments).map((segment) => ({
      id: segment.id,
      start: segment.timestamp ?? 0,
      end: segment.endTime,
      text: segment.text,
      speaker: segment.speaker,
      confidence: segment.confidence,
    }));
    return buildTableRows(lines, (speaker) => sideOf(sides, speaker));
  }, [segments, sides]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 72,
    overscan: 8,
    getItemKey: (index) => rows[index]?.key ?? index,
  });

  // Follow the playhead, the way the chat view does.
  useEffect(() => {
    if (!followActiveSegment || !activeSegmentId) return;
    const index = rowIndexOf(rows, activeSegmentId);
    if (index < 0) return;
    const container = scrollRef.current;
    const element = container?.querySelector<HTMLElement>(
      `[id="table-line-${CSS.escape(activeSegmentId)}"]`,
    );
    if (container && element) {
      const line = element.getBoundingClientRect();
      const view = container.getBoundingClientRect();
      if (line.top >= view.top && line.bottom <= view.bottom) return;
    }
    virtualizer.scrollToIndex(index, { align: 'center' });
    // `virtualizer` is stable for the life of the list.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeSegmentId, followActiveSegment, rows]);

  // Load the next page when the end comes into view.
  const handleScroll = useCallback(() => {
    const element = scrollRef.current;
    if (!element || !onLoadMore || !hasMore || isLoadingMore) return;
    if (element.scrollHeight - element.scrollTop - element.clientHeight < 200) {
      onLoadMore();
    }
  }, [onLoadMore, hasMore, isLoadingMore]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div
        className={`${GRID} border-b border-[var(--af-border)] pb-2 pl-6 pr-4 pt-1 text-[11px] font-semibold uppercase tracking-wide text-[var(--af-text-3)]`}
      >
        <div>{t('tableColumnTime')}</div>
        <div>{t('tableColumnClient')}</div>
        <div>{userName || t('tableColumnHost')}</div>
      </div>

      <div ref={scrollRef} onScroll={handleScroll} className="min-h-0 flex-1 overflow-y-auto px-4">
        {rows.length === 0 ? (
          <p className="mt-8 text-center text-sm text-[var(--af-text-3)]">
            {tr('startRecordingToSeeTranscription')}
          </p>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: 'relative', width: '100%' }}>
            {virtualizer.getVirtualItems().map((item) => {
              const row = rows[item.index];
              return (
                <div
                  key={item.key}
                  data-index={item.index}
                  ref={virtualizer.measureElement}
                  style={{
                    position: 'absolute',
                    top: 0,
                    left: 0,
                    width: '100%',
                    transform: `translateY(${item.start}px)`,
                  }}
                >
                  <Row
                    row={row}
                    userName={userName}
                    // Only the row holding the active line gets it, so moving
                    // the playhead re-renders one row rather than all of them.
                    activeSegmentId={
                      activeSegmentId && row.ids.includes(activeSegmentId) ? activeSegmentId : null
                    }
                    onSeekTo={onSeekTo}
                    onRenameSpeaker={onRenameSpeaker}
                  />
                </div>
              );
            })}
          </div>
        )}

        {(hasMore || isLoadingMore) && rows.length > 0 && (
          <div className="flex items-center justify-center py-4 text-sm text-[var(--af-text-3)]">
            {isLoadingMore
              ? tr('loadingMore')
              : totalCount > 0
                ? tr('showingSegments', { loaded: loadedCount, total: totalCount })
                : null}
          </div>
        )}
      </div>
    </div>
  );
}

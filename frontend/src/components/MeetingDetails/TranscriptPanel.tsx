"use client";

/**
 * Transcript column for the MEETING-DETAILS screen — the wide middle column.
 *
 * ⚠️ There are TWO components named `TranscriptPanel`. This is the
 * meeting-details one; the live recording screen uses
 * `app/_components/TranscriptPanel.tsx`. Editing the wrong file is a common
 * trap — it compiles and appears to do nothing.
 *
 * Both data paths feeding the virtualized view must carry `speaker` or speaker
 * labels vanish here:
 *   - paginated  → `segments` prop, built by `hooks/usePaginatedTranscripts.ts`
 *   - otherwise  → converted inline from `transcripts` below
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useLocale, useTranslations } from 'next-intl';
import { Transcript, TranscriptSegmentData } from '@/types';
import { Calendar, Clock, MessagesSquare, Table2 } from 'lucide-react';
import { SpeakerRenameDialog } from './SpeakerRenameDialog';
import {
  VirtualizedTranscriptView,
  mergeAdjacentSameSpeaker,
} from '@/components/VirtualizedTranscriptView';
import { TranscriptButtonGroup } from './TranscriptButtonGroup';
import { RecordingPlayer, RecordingPlayerHandle } from './RecordingPlayer';
import { MeetingClientBadge } from '@/components/MeetingClientBadge';
import { TranscriptTableView } from './TranscriptTableView';
import { createPlayhead } from '@/lib/playhead';

type TranscriptLayout = 'chat' | 'table';

/** A reading preference, kept per machine rather than per meeting. */
const LAYOUT_STORAGE_KEY = 'transcript_layout';

interface TranscriptPanelProps {
  transcripts: Transcript[];
  title?: string;
  createdAt?: string;
  customPrompt: string;
  onPromptChange: (value: string) => void;
  onCopyTranscript: () => void;
  onOpenExport?: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  isRecording: boolean;
  disableAutoScroll?: boolean;

  // Optional pagination props (when using virtualization)
  usePagination?: boolean;
  segments?: TranscriptSegmentData[];
  hasMore?: boolean;
  isLoadingMore?: boolean;
  totalCount?: number;
  loadedCount?: number;
  onLoadMore?: () => void;

  // Retranscription props
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;
  onSpeakerRenamed?: (rename: { from: string; to: string; count: number; removedName: boolean }) => void;
}

function fmtDate(d: Date, locale: string): string {
  return d.toLocaleDateString(locale, { month: 'long', day: 'numeric', year: 'numeric' });
}
function fmtTime(d: Date, locale: string): string {
  return d.toLocaleTimeString(locale, { hour: 'numeric', minute: '2-digit' });
}

export function TranscriptPanel({
  transcripts,
  title,
  createdAt,
  customPrompt,
  onPromptChange,
  onCopyTranscript,
  onOpenExport,
  onOpenMeetingFolder,
  isRecording,
  disableAutoScroll = false,
  usePagination = false,
  segments,
  hasMore,
  isLoadingMore,
  totalCount,
  loadedCount,
  onLoadMore,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
  onSpeakerRenamed,
}: TranscriptPanelProps) {
  const t = useTranslations('meetingDetails');
  const locale = useLocale();
  const [renameTarget, setRenameTarget] = useState<string | null>(null);
  // Which line the recording is at, and whether it is moving. Only these two
  // come back from the player; the playhead itself stays inside it.
  const [activeSegmentId, setActiveSegmentId] = useState<string | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  // The moving line is drawn only when there is a recording to move with.
  const [hasAudio, setHasAudio] = useState(false);
  const playerRef = useRef<RecordingPlayerHandle>(null);
  const seekTo = useCallback((seconds: number) => playerRef.current?.seek(seconds), []);
  // One per meeting: a new recording starts from the top.
  const playhead = useMemo(() => createPlayhead(), [meetingId]);

  const [layout, setLayout] = useState<TranscriptLayout>('chat');
  useEffect(() => {
    try {
      if (localStorage.getItem(LAYOUT_STORAGE_KEY) === 'table') setLayout('table');
    } catch {
      // Storage can be unavailable; the chat layout is the default anyway.
    }
  }, []);
  const chooseLayout = useCallback((next: TranscriptLayout) => {
    setLayout(next);
    try {
      localStorage.setItem(LAYOUT_STORAGE_KEY, next);
    } catch {
      // Not remembered this time; nothing else depends on it.
    }
  }, []);
  // The table is laid out by meeting and by who spoke, and neither is settled
  // while recording, so the live screen keeps the chat.
  const canShowTable = !isRecording && !!meetingId;
  const showTable = canShowTable && layout === 'table';

  const convertedSegments = useMemo(() => {
    if (usePagination && segments) return segments;
    return transcripts.map(t => ({
      id: t.id,
      timestamp: t.audio_start_time ?? 0,
      endTime: t.audio_end_time,
      text: t.text,
      confidence: t.confidence,
      speaker: t.speaker,
    }));
  }, [transcripts, usePagination, segments]);

  // Speaker roles are asked for again whenever the set of speakers changes —
  // after a rename, or a rerun of recognition or diarization.
  const speakersKey = useMemo(
    () => Array.from(new Set(convertedSegments.map((s) => s.speaker ?? ''))).sort().join('\n'),
    [convertedSegments],
  );

  // The player has to name the same lines the transcript draws, and the
  // transcript draws one bubble per turn rather than per VAD fragment.
  const displayedSegments = useMemo(
    () => mergeAdjacentSameSpeaker(convertedSegments),
    [convertedSegments],
  );

  // Date + time range for the header. Start comes from the meeting timestamp;
  // the end is derived from the furthest transcript position we know about.
  const { dateLabel, timeLabel } = useMemo(() => {
    const start = createdAt ? new Date(createdAt) : null;
    if (!start || isNaN(start.getTime())) return { dateLabel: '', timeLabel: '' };
    const durationSec = convertedSegments.reduce(
      (max, s) => Math.max(max, (s as any).endTime ?? s.timestamp ?? 0),
      0,
    );
    const end = durationSec > 0 ? new Date(start.getTime() + durationSec * 1000) : null;
    return {
      dateLabel: fmtDate(start, locale),
      timeLabel: end ? `${fmtTime(start, locale)} — ${fmtTime(end, locale)}` : fmtTime(start, locale),
    };
  }, [createdAt, convertedSegments, locale]);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-[var(--af-bg)]">
      {/* Header: title + date/time */}
      <div className="min-w-0 px-4 pt-5 sm:px-6 sm:pt-6 lg:px-8">
        <h1 className="truncate text-xl font-bold text-[var(--af-text)] sm:text-2xl">
          {title || t('untitledMeeting')}
        </h1>
        <div className="mt-2 flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1 text-sm text-[var(--af-text-2)]">
          <MeetingClientBadge meetingId={meetingId} />
          {dateLabel && (
            <span className="inline-flex min-w-0 items-center gap-1.5">
              <Calendar size={15} className="shrink-0 text-[var(--af-text-3)]" />
              <span className="truncate">{dateLabel}</span>
            </span>
          )}
          {timeLabel && (
            <span className="inline-flex min-w-0 items-center gap-1.5">
              <Clock size={15} className="shrink-0 text-[var(--af-text-3)]" />
              <span className="truncate">{timeLabel}</span>
            </span>
          )}
        </div>
      </div>

      {/* The action container owns its responsive breakpoint, since this column
          can be narrow even when the overall window is wide. */}
      <div className="mt-4 flex min-w-0 items-center gap-2 border-b border-[var(--af-border)] px-4 sm:mt-5 sm:gap-3 sm:px-6 lg:px-8">
        <span className="relative -mb-px shrink-0 py-2 text-sm font-medium text-[var(--af-accent)]">
          {t('transcriptTab')}
          <span className="absolute inset-x-0 -bottom-px h-0.5 rounded-full bg-[var(--af-accent)]" />
        </span>
        {canShowTable && (
          <div
            role="radiogroup"
            aria-label={t('transcriptViewLabel')}
            className="flex shrink-0 items-center rounded-md border border-[var(--af-border)] p-0.5"
          >
            {([
              ['chat', MessagesSquare, t('transcriptViewChat')],
              ['table', Table2, t('transcriptViewTable')],
            ] as const).map(([value, Icon, label]) => (
              <button
                key={value}
                type="button"
                role="radio"
                aria-checked={layout === value}
                aria-label={label}
                title={label}
                onClick={() => chooseLayout(value)}
                className={`rounded px-1.5 py-1 transition-colors ${
                  layout === value
                    ? 'bg-[var(--af-panel-2)] text-[var(--af-accent)]'
                    : 'text-[var(--af-text-3)] hover:text-[var(--af-text)]'
                }`}
              >
                <Icon size={15} />
              </button>
            ))}
          </div>
        )}
        <div className="transcript-actions-container ml-auto min-w-0 flex-1 overflow-x-auto overscroll-x-contain py-1 no-scrollbar">
          <TranscriptButtonGroup
            transcriptCount={usePagination ? (totalCount ?? convertedSegments.length) : (transcripts?.length || 0)}
            onCopyTranscript={onCopyTranscript}
            onOpenExport={onOpenExport}
            onOpenMeetingFolder={onOpenMeetingFolder}
            meetingId={meetingId}
            meetingFolderPath={meetingFolderPath}
            onRefetchTranscripts={onRefetchTranscripts}
          />
        </div>
      </div>

      <SpeakerRenameDialog
        open={renameTarget !== null}
        speaker={renameTarget}
        meetingId={meetingId}
        onOpenChange={(open) => !open && setRenameTarget(null)}
        onRenamed={async (rename) => {
          await onRefetchTranscripts?.();
          onSpeakerRenamed?.(rename);
        }}
      />

      {/* Transcript content */}
      {/* The table's ruler sits flush with the panel edge, so it takes no side padding. */}
      <div className={`flex-1 overflow-hidden pb-4 ${showTable ? "" : "px-4"}`}>
        {showTable ? (
          <TranscriptTableView
            segments={convertedSegments}
            meetingId={meetingId}
            speakersKey={speakersKey}
            onRenameSpeaker={setRenameTarget}
            activeSegmentId={activeSegmentId}
            onSeekTo={seekTo}
            followActiveSegment={isPlaying}
            playhead={hasAudio ? playhead : undefined}
            hasMore={hasMore}
            isLoadingMore={isLoadingMore}
            totalCount={totalCount}
            loadedCount={loadedCount}
            onLoadMore={onLoadMore}
          />
        ) : (
        <VirtualizedTranscriptView
          onRenameSpeaker={meetingId ? setRenameTarget : undefined}
          activeSegmentId={activeSegmentId}
          onSeekTo={isRecording ? undefined : seekTo}
          followActiveSegment={isPlaying}
          segments={convertedSegments}
          isRecording={isRecording}
          isPaused={false}
          isProcessing={false}
          isStopping={false}
          enableStreaming={false}
          showConfidence={true}
          disableAutoScroll={disableAutoScroll}
          hasMore={hasMore}
          isLoadingMore={isLoadingMore}
          totalCount={totalCount}
          loadedCount={loadedCount}
          onLoadMore={onLoadMore}
        />
        )}
      </div>

      {/* Playing a recording only makes sense once it exists; the player takes
          up no room when the meeting has no audio to play. */}
      {!isRecording && (
        <RecordingPlayer
          ref={playerRef}
          meetingFolderPath={meetingFolderPath}
          segments={displayedSegments}
          onActiveSegmentChange={setActiveSegmentId}
          onPlayingChange={setIsPlaying}
          playhead={playhead}
          onAvailabilityChange={setHasAudio}
        />
      )}
    </div>
  );
}

'use client';

/**
 * The meeting page's two columns, transcript and summary, with a divider
 * between them. Either column can be hidden; a narrow strip on its side
 * brings it back. Sizes and visibility are remembered.
 *
 * A hidden column stays mounted, so the player keeps playing and the summary
 * editor keeps its unsaved state.
 */

import React, { useRef } from 'react';
import { useTranslations } from 'next-intl';
import { PanelLeftClose, PanelLeftOpen, PanelRightClose, PanelRightOpen } from 'lucide-react';
import { ResizeHandle } from '@/components/ResizeHandle';
import { usePanelLayout } from '@/components/PanelLayoutProvider';
import { SPLIT_DEFAULT, SPLIT_MAX, SPLIT_MIN, clampSplit, isPaneShown, splitAt } from '@/lib/panel-layout';

const iconButton =
  'rounded-md p-1 text-[var(--af-text-3)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)]';

function Rail({
  side,
  label,
  showLabel,
  onShow,
}: {
  side: 'left' | 'right';
  label: string;
  showLabel: string;
  onShow: () => void;
}) {
  const Icon = side === 'left' ? PanelLeftOpen : PanelRightOpen;
  return (
    <button
      type="button"
      onClick={onShow}
      title={showLabel}
      aria-label={showLabel}
      // A strip across the page when the columns stack, down its side otherwise.
      className={`flex h-9 shrink-0 items-center justify-center gap-2 border-[var(--af-border)] bg-[var(--af-panel)] text-[var(--af-text-3)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--af-accent)] md:h-auto md:w-8 md:flex-col md:justify-start md:gap-3 md:py-3 ${
        side === 'left' ? 'border-b md:border-b-0 md:border-r' : 'border-t md:border-t-0 md:border-l'
      }`}
    >
      <Icon size={16} />
      <span className="text-xs font-medium md:[writing-mode:vertical-rl]">{label}</span>
    </button>
  );
}

export function MeetingSplitLayout({
  transcript,
  summary,
}: {
  transcript: React.ReactNode;
  summary: React.ReactNode;
}) {
  const t = useTranslations('meetingDetails');
  const { layout, setSplit, togglePane } = usePanelLayout();
  const row = useRef<HTMLDivElement>(null);
  const showTranscript = isPaneShown(layout.panes, 'transcript');
  const showSummary = isPaneShown(layout.panes, 'summary');
  const both = showTranscript && showSummary;

  const splitFor = (x: number) => {
    const box = row.current?.getBoundingClientRect();
    return box ? splitAt(x, box.left, box.width) : layout.split;
  };
  const preview = (split: number) => row.current?.style.setProperty('--split', String(split));

  return (
    <div
      ref={row}
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-[var(--af-bg)] md:flex-row"
      style={{ '--split': String(layout.split) } as React.CSSProperties}
    >
      {!showTranscript && (
        <Rail side="left" label={t('transcriptPane')} showLabel={t('showTranscriptPane')} onShow={() => togglePane('transcript')} />
      )}

      <div
        className={`min-h-0 min-w-0 flex-col md:min-w-[280px] ${showTranscript ? 'flex' : 'hidden'}`}
        style={{ flex: both ? 'var(--split) 1 0' : '1 1 0' }}
      >
        {transcript}
      </div>

      {both && (
        <div className="relative hidden shrink-0 md:flex">
          <ResizeHandle
            label={t('resizePanes')}
            valueNow={layout.split * 100}
            valueMin={SPLIT_MIN * 100}
            valueMax={SPLIT_MAX * 100}
            onDrag={(x) => preview(splitFor(x))}
            onDragEnd={(x) => {
              const split = splitFor(x);
              preview(split);
              setSplit(split);
            }}
            onStep={(direction, big) => setSplit(clampSplit(layout.split + direction * (big ? 0.1 : 0.02)))}
            onReset={() => setSplit(SPLIT_DEFAULT)}
            className="-mx-1 h-full"
          />
          {/* Hide either side from the divider. */}
          <div className="absolute left-1/2 top-2 z-30 flex -translate-x-1/2 flex-col gap-0.5 rounded-lg border border-[var(--af-border)] bg-[var(--af-panel)] p-0.5 shadow-sm">
            <button
              type="button"
              onClick={() => togglePane('transcript')}
              title={t('hideTranscriptPane')}
              aria-label={t('hideTranscriptPane')}
              className={iconButton}
            >
              <PanelLeftClose size={14} />
            </button>
            <button
              type="button"
              onClick={() => togglePane('summary')}
              title={t('hideSummaryPane')}
              aria-label={t('hideSummaryPane')}
              className={iconButton}
            >
              <PanelRightClose size={14} />
            </button>
          </div>
        </div>
      )}

      <div
        className={`min-h-0 min-w-0 flex-col md:min-w-[280px] ${showSummary ? 'flex' : 'hidden'}`}
        style={{ flex: both ? 'calc(1 - var(--split)) 1 0' : '1 1 0' }}
      >
        {summary}
      </div>

      {!showSummary && (
        <Rail side="right" label={t('summaryPane')} showLabel={t('showSummaryPane')} onShow={() => togglePane('summary')} />
      )}
    </div>
  );
}

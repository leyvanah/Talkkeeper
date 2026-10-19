'use client';

/**
 * Writes the automatic summary of a recording whose processing finished while
 * its page was not open.
 *
 * The summary used to start only from the meeting page. With several
 * recordings in the queue the window has long moved on to the next one, so a
 * processed recording sat without its summary until the owner happened to
 * open it. This does what the page would have done, with the same settings and
 * the same template, and renders nothing.
 */

import { useEffect, useMemo, useRef } from 'react';
import { useConfig } from '@/contexts/ConfigContext';
import { usePostCall } from '@/contexts/PostCallContext';
import { useTemplates } from '@/hooks/meeting-details/useTemplates';
import { useSummaryGeneration } from '@/hooks/meeting-details/useSummaryGeneration';
import { startAutoSummary, summaryStartKey } from '@/lib/auto-summary';

const ignore = () => {};

export function BackgroundSummary({
  meetingId,
  onDone,
}: {
  meetingId: string;
  /** Called once the summary has started, or will not start from here. */
  onDone: () => void;
}) {
  const { modelConfig, isAutoSummary } = useConfig();
  const { selectedTemplate } = useTemplates();
  const { release } = usePostCall();
  const meeting = useMemo(() => ({ id: meetingId }), [meetingId]);
  const summaryGeneration = useSummaryGeneration({
    meeting,
    // The summary reads every line from the database itself.
    transcripts: [],
    modelConfig,
    isModelConfigLoading: false,
    selectedTemplate,
    setAiSummary: ignore,
  });

  const started = useRef(false);
  const onDoneRef = useRef(onDone);
  onDoneRef.current = onDone;
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    void (async () => {
      try {
        // Without automatic summaries, or without a model chosen, the page
        // decides as it always has once the meeting is opened.
        if (!isAutoSummary || !modelConfig?.model) return;
        if (sessionStorage.getItem(summaryStartKey(meetingId))) {
          release(meetingId);
          return;
        }
        const accepted = await startAutoSummary(meetingId, () =>
          summaryGeneration.handleGenerateSummary(''),
        );
        if (accepted) {
          sessionStorage.setItem(summaryStartKey(meetingId), 'started');
          release(meetingId);
        }
      } catch (error) {
        console.error('Background summary did not start:', error);
      } finally {
        onDoneRef.current();
      }
    })();
    // Started once per meeting; the settings are read as they are by then.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return null;
}

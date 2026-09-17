/**
 * Corrections to a saved transcript.
 *
 * A displayed line may be several stored lines merged for reading; its `ids`
 * list them. An edit is applied to the first and takes the rest over; a
 * removal removes them all. Either way re-recognition leaves the result alone.
 */

import { invoke } from '@tauri-apps/api/core';
import { TranscriptSegmentData } from '@/types';

function storedIds(line: Pick<TranscriptSegmentData, 'id' | 'ids'>): string[] {
  return line.ids?.length ? line.ids : [line.id];
}

export function editTranscriptLine(
  meetingId: string,
  line: Pick<TranscriptSegmentData, 'id' | 'ids'>,
  text: string,
): Promise<unknown> {
  const [transcriptId, ...absorbedIds] = storedIds(line);
  return invoke('api_edit_transcript_line', { meetingId, transcriptId, text, absorbedIds });
}

export function removeTranscriptLine(
  meetingId: string,
  line: Pick<TranscriptSegmentData, 'id' | 'ids'>,
): Promise<void> {
  return invoke<void>('api_remove_transcript_lines', {
    meetingId,
    transcriptIds: storedIds(line),
  });
}

/** A displayed line, as far as corrections need to know it. */
export type LineRef = Pick<TranscriptSegmentData, 'id' | 'ids'>;

/** Correcting and removing lines; absent where the transcript cannot change. */
export interface LineEdits {
  onEditLine: (line: LineRef, text: string) => Promise<void>;
  onRemoveLine: (line: LineRef) => Promise<void>;
}

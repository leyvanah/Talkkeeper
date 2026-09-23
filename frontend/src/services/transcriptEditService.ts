/**
 * Corrections to a saved transcript.
 *
 * A displayed line may be several stored lines merged for reading; its `ids`
 * list them. An edit is applied to the first and takes the rest over; a
 * removal removes them all; a split cuts them into two lines; a new speaker
 * is given to all of them. Every way, re-recognition leaves the result alone.
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

export function setTranscriptLineSpeaker(
  meetingId: string,
  line: Pick<TranscriptSegmentData, 'id' | 'ids'>,
  speaker: string,
): Promise<void> {
  return invoke<void>('api_set_transcript_speaker', {
    meetingId,
    transcriptIds: storedIds(line),
    speaker,
  });
}

export function splitTranscriptLine(
  meetingId: string,
  line: Pick<TranscriptSegmentData, 'id' | 'ids'>,
  firstText: string,
  secondText: string,
): Promise<unknown> {
  return invoke('api_split_transcript_line', {
    meetingId,
    transcriptIds: storedIds(line),
    firstText,
    secondText,
  });
}

/** Join displayed lines into one, in the order they were said. */
export function mergeTranscriptLines(
  meetingId: string,
  lines: Array<Pick<TranscriptSegmentData, 'id' | 'ids'>>,
): Promise<unknown> {
  return invoke('api_merge_transcript_lines', {
    meetingId,
    transcriptIds: lines.flatMap(storedIds),
  });
}

/** Whether a meeting has corrections to undo or redo in this session. */
export interface EditHistory {
  canUndo: boolean;
  canRedo: boolean;
}

export function transcriptEditHistory(meetingId: string): Promise<EditHistory> {
  return invoke<EditHistory>('api_transcript_edit_history', { meetingId });
}

export function undoTranscriptEdit(meetingId: string): Promise<EditHistory> {
  return invoke<EditHistory>('api_undo_transcript_edit', { meetingId });
}

export function redoTranscriptEdit(meetingId: string): Promise<EditHistory> {
  return invoke<EditHistory>('api_redo_transcript_edit', { meetingId });
}

/** A displayed line, as far as corrections need to know it. */
export type LineRef = Pick<TranscriptSegmentData, 'id' | 'ids'>;

/** Correcting and removing lines; absent where the transcript cannot change. */
export interface LineEdits {
  onEditLine: (line: LineRef, text: string) => Promise<void>;
  onRemoveLine: (line: LineRef) => Promise<void>;
  /** Give the line to another speaker. */
  onSetSpeaker: (line: LineRef, speaker: string) => Promise<void>;
  /** Cut the line in two; each half keeps its stretch of the recording. */
  onSplitLine: (line: LineRef, first: string, second: string) => Promise<void>;
  /** Join the line with the one shown after it. */
  onMergeLines: (line: LineRef, next: LineRef) => Promise<void>;
  /** Every speaker a line can be given to (raw labels). */
  speakers: string[];
  /** A label for a speaker the meeting does not have yet. */
  freshSpeaker: string;
}

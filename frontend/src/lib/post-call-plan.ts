/**
 * What happens to a recording after it stops, decided by what it kept.
 *
 * The refining pass and the speaker model both read the saved audio. With
 * "Save audio recordings" off there is none, and asking for the pass anyway
 * ended every such recording in an error naming the recording folder.
 */

export type PostCallPlan =
  /** Audio is on disk: refine the transcript and tell the speakers apart. */
  | 'refine'
  /** No audio, but the live transcript has lines: keep them as they are. */
  | 'keep-live'
  /** Neither audio nor live text: there is nothing to keep or refine. */
  | 'nothing-kept';

export interface RecordingKept {
  /** Whether the meeting folder holds audio; `null` when it could not be told. */
  hasAudio: boolean | null;
  /** Lines in the stored transcript; `null` when they could not be counted. */
  lineCount: number | null;
}

export function planPostCall({ hasAudio, lineCount }: RecordingKept): PostCallPlan {
  // Unknown is treated as present: the pass then runs as it always has, and a
  // real failure is still reported.
  if (hasAudio !== false) return 'refine';
  if (lineCount === null || lineCount > 0) return 'keep-live';
  return 'nothing-kept';
}

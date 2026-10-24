/**
 * When the question after a recording may be put on screen.
 *
 * A recording started in the two seconds between saving the previous one and
 * leaving for its page keeps the window, and the previous meeting's work
 * begins right on the recording screen. Its question ("how many speakers?", or "try again?"
 * after a failure) is a dialog that cannot be dismissed: asked then, it would
 * cover the live transcript and notes until answered. The run waits instead,
 * and the dialog comes once the recording has stopped.
 */

export interface RecordingNow {
  /** The backend reports a recording. */
  isRecording: boolean;
  /** The window's recording status (`RecordingStatus` values). */
  status: string;
}

// `starting`: the backend may not report the new recording yet.
const RECORDING_STATUSES = new Set(['starting', 'recording']);

export function mayAskNow({ isRecording, status }: RecordingNow): boolean {
  return !isRecording && !RECORDING_STATUSES.has(status);
}

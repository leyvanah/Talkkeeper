/**
 * Whether the home screen's title is the current recording's name.
 *
 * Only while there is a recording to name: being recorded, being stopped or
 * being saved. Afterwards the screen is where a new recording starts, though
 * the last name stays in the transcript context until the next recording
 * replaces it. Starting does not count yet: the new name is set a moment later,
 * and until then the old one would show.
 */
export function showsRecordingName(isRecording: boolean, status: string): boolean {
  return isRecording || ['stopping', 'processing', 'saving', 'completed'].includes(status);
}

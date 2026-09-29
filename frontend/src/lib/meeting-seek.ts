/**
 * Asking the meeting's player to go to a moment, from anywhere on the page.
 *
 * The player lives inside the transcript column; the notes are in the panel
 * beside it. A window event keeps the two apart instead of threading a ref
 * through the page for one call.
 */

const EVENT = 'meeting-seek';

interface SeekRequest {
  meetingId: string;
  seconds: number;
}

export function requestMeetingSeek(meetingId: string, seconds: number): void {
  window.dispatchEvent(new CustomEvent<SeekRequest>(EVENT, { detail: { meetingId, seconds } }));
}

/** Calls `seek` for requests about `meetingId`; returns the unsubscribe. */
export function onMeetingSeek(meetingId: string, seek: (seconds: number) => void): () => void {
  const listener = (event: Event) => {
    const { detail } = event as CustomEvent<SeekRequest>;
    if (detail?.meetingId === meetingId && Number.isFinite(detail.seconds)) seek(detail.seconds);
  };
  window.addEventListener(EVENT, listener);
  return () => window.removeEventListener(EVENT, listener);
}

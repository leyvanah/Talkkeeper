/**
 * The client chosen for the recording that is running right now.
 *
 * A meeting row does not exist until the recording stops, so the choice has to
 * survive from "record" to "saved" outside the database. sessionStorage is
 * where this screen already keeps that kind of thing (the recording folder and
 * meeting name travel the same way), and it clears itself if the app is closed
 * mid-recording — which is the right default: a recovered recording should be
 * filed deliberately, not by a stale choice from days ago.
 */

const PENDING_RECORDING_CLIENT_KEY = 'pending_recording_client_id';

export function getPendingRecordingClient(): string | null {
  try {
    const stored = sessionStorage.getItem(PENDING_RECORDING_CLIENT_KEY);
    return stored && stored.length > 0 ? stored : null;
  } catch {
    return null;
  }
}

export function setPendingRecordingClient(clientId: string | null): void {
  try {
    if (clientId) sessionStorage.setItem(PENDING_RECORDING_CLIENT_KEY, clientId);
    else sessionStorage.removeItem(PENDING_RECORDING_CLIENT_KEY);
  } catch {
    /* the recording still saves; it just lands in "Unassigned" */
  }
}

export function clearPendingRecordingClient(): void {
  setPendingRecordingClient(null);
}

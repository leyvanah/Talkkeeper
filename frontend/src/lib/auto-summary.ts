/**
 * The automatic summary after a recording, shared by the meeting page and the
 * background runner that writes it while the page is not open.
 *
 * Both may reach the same meeting — the owner can open it while the summary is
 * starting in the background — so they share one attempt per meeting and one
 * "already started" mark, and the summary is asked for once.
 */

/** The attempt under way for a meeting; resolves to whether the backend took it. */
export const autoSummaryInFlight = new Map<string, Promise<boolean>>();

/** Set once a recording's automatic summary has started; survives a reload. */
export const summaryStartKey = (meetingId: string) => `post-call-summary-started:${meetingId}`;

/**
 * Starts the summary through `generate`, or joins an attempt already under way,
 * and returns whether the backend accepted it.
 */
export async function startAutoSummary(
  meetingId: string,
  generate: () => Promise<boolean>,
): Promise<boolean> {
  let attempt = autoSummaryInFlight.get(meetingId);
  if (!attempt) {
    attempt = generate();
    autoSummaryInFlight.set(meetingId, attempt);
  }
  try {
    return await attempt;
  } finally {
    if (autoSummaryInFlight.get(meetingId) === attempt) {
      autoSummaryInFlight.delete(meetingId);
    }
  }
}

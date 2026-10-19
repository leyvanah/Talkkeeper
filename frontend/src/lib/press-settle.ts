/**
 * A button whose command returns almost at once (pause/resume takes a couple
 * of milliseconds) would be enabled again before the second click of a double
 * click lands, and that click would undo the first. Holding the button busy
 * for a short minimum makes a double click one command, and lets the spinner
 * be seen at all.
 */

/** The least time a press keeps its button busy. */
export const PRESS_SETTLE_MS = 400;

/** How much longer a press made at `startedAt` should stay busy at `now`. */
export function settleRemaining(
  startedAt: number,
  now: number,
  settleMs: number = PRESS_SETTLE_MS,
): number {
  return Math.min(settleMs, Math.max(0, settleMs - (now - startedAt)));
}

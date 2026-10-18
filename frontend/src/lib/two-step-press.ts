/**
 * A button that has to be pressed twice: the first press arms it and asks
 * ("Stop?"), a second one within the window does the thing. Used where an
 * action cannot be undone and there is no room for a dialog (the compact bar).
 */

/** How long an armed button waits for the second press. */
export const ARM_WINDOW_MS = 3000;

/**
 * Presses this soon after arming are ignored, so a double click — a slip, not
 * a decision — does not arm and confirm in one go.
 */
export const CONFIRM_GRACE_MS = 400;

export type TwoStepOutcome = 'armed' | 'confirmed' | 'ignored';

/** What a press at `now` does, given when the button was armed (`null`: it is not). */
export function pressTwoStep(
  armedAt: number | null,
  now: number,
  windowMs: number = ARM_WINDOW_MS,
  graceMs: number = CONFIRM_GRACE_MS,
): TwoStepOutcome {
  if (armedAt === null) return 'armed';
  const since = now - armedAt;
  if (since < 0 || since >= windowMs) return 'armed';
  if (since < graceMs) return 'ignored';
  return 'confirmed';
}

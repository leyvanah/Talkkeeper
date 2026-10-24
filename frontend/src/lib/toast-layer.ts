/**
 * Toasts over a modal dialog.
 *
 * A modal Radix dialog takes the pointer away from everything outside it and
 * closes on a press outside. Toasts render outside every dialog, so a toast
 * that came up over one could not be pressed — its close button and its action
 * ("Open meeting", "Undo") did nothing until it timed out. The stylesheet gives
 * toasts the pointer back; this keeps a press on a toast from counting as a
 * press outside the dialog.
 */

/** Anything that can look up the element tree, as a DOM element can. */
interface Closest {
  closest(selector: string): unknown;
}

/** The attribute sonner puts on its list of toasts. */
export const TOASTER_SELECTOR = '[data-sonner-toaster]';

/** Whether an event's target lies inside the toasts. */
export function isInsideToaster(target: unknown): boolean {
  if (!target || typeof (target as Closest).closest !== 'function') return false;
  return !!(target as Closest).closest(TOASTER_SELECTOR);
}

/**
 * Wraps a Radix "outside" handler so that a press on a toast leaves the dialog
 * open; anything else goes to the handler the dialog was given.
 */
export function keepOpenForToasts<E extends { target: EventTarget | null; preventDefault(): void }>(
  handler?: (event: E) => void,
): (event: E) => void {
  return (event) => {
    if (isInsideToaster(event.target)) {
      event.preventDefault();
      return;
    }
    handler?.(event);
  };
}

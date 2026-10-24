/**
 * Wraps an action so that a call made while an earlier one is still running
 * does nothing: a double click on "Delete" deletes once. Resolves to whether
 * this call ran the action. A failed action frees the way for the next call.
 */
export function exclusive<A extends unknown[]>(
  action: (...args: A) => Promise<void> | void,
): (...args: A) => Promise<boolean> {
  let running = false;
  return async (...args: A) => {
    if (running) return false;
    running = true;
    try {
      await action(...args);
      return true;
    } finally {
      running = false;
    }
  };
}

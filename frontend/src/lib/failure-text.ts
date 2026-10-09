/**
 * The original text of a failure, for the application log.
 *
 * A Tauri command rejects with a plain string, a JavaScript error is an
 * `Error`, and some layers pass `{ message }` or `{ error }` objects. The owner
 * never sees this text — the window shows a phrase in the interface language —
 * but it is what explains the failure afterwards, so it is pulled out of
 * whichever shape it came in.
 */
export function failureText(error: unknown): string {
  if (typeof error === 'string') return error.trim() || 'unknown error'
  if (error instanceof Error) return error.message.trim() || error.name
  if (error && typeof error === 'object') {
    const shaped = error as { message?: unknown; error?: unknown }
    if (typeof shaped.message === 'string' && shaped.message.trim()) return shaped.message.trim()
    if (typeof shaped.error === 'string' && shaped.error.trim()) return shaped.error.trim()
    try {
      return JSON.stringify(error)
    } catch {
      // A cyclic object: fall through to the generic text.
    }
  }
  if (error === undefined || error === null) return 'unknown error'
  return String(error)
}

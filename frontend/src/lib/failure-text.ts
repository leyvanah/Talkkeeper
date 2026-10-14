/**
 * The original text of a failure, for the application log.
 *
 * A Tauri command rejects with a plain string, a JavaScript error is an
 * `Error`, and some layers pass `{ message }` or `{ error }` objects. The owner
 * never sees this text — the window shows a phrase in the interface language —
 * but it is what explains the failure afterwards, so it is pulled out of
 * whichever shape it came in.
 *
 * Any other object is logged by its field names only: the log is not
 * encrypted, and such an object could carry archive data (a command reply, a
 * piece of a meeting), so its values stay out.
 */
export function failureText(error: unknown): string {
  if (typeof error === 'string') return error.trim() || 'unknown error'
  if (error instanceof Error) return error.message.trim() || error.name
  if (error && typeof error === 'object') {
    const shaped = error as { message?: unknown; error?: unknown }
    if (typeof shaped.message === 'string' && shaped.message.trim()) return shaped.message.trim()
    if (typeof shaped.error === 'string' && shaped.error.trim()) return shaped.error.trim()
    if (Array.isArray(error)) return `[array of ${error.length}]`
    return `{${Object.keys(error).join(', ')}}`
  }
  if (error === undefined || error === null) return 'unknown error'
  return String(error)
}

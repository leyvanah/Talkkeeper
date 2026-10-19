/**
 * What the window does when an action fails: tells the owner in their own
 * language what did not work, and keeps the original error text in the
 * application log (`logs/meetily.log`) instead of on screen.
 *
 * Backend errors are English, technical and sometimes carry paths; shown as
 * they were, they told a Russian-speaking owner nothing and hid the phrases
 * written for them. `console.error` alone was no answer either: a release
 * build has no console anyone looks at.
 */

import { invoke } from '@tauri-apps/api/core'
import { toast, type ExternalToast } from 'sonner'
import { failureText } from './failure-text'

/**
 * Writes the original error to the application log. `action` is a fixed name
 * for what was being done (`speaker-rename`), not a sentence: the backend keeps
 * only letters, digits, `-`, `_` and `.` of it.
 */
export function logFailure(action: string, error: unknown): void {
  const detail = failureText(error)
  console.error(`[${action}]`, error)
  invoke('log_frontend_failure', { action, detail }).catch(() => {
    // Logging must never turn one failure into two; the console has it.
  })
}

/**
 * Shows `message` as an error toast and logs the original error. `options`
 * may carry a toast id or a description written for the owner — what to do
 * next — never the error text itself.
 */
export function toastFailure(
  message: string,
  action: string,
  error: unknown,
  options?: ExternalToast,
): void {
  toast.error(message, options)
  logFailure(action, error)
}

/**
 * The printed sheet for the recovery code, and how the window knows the print
 * dialog actually opened.
 *
 * The sheet is printed from a hidden frame inside the page: the app's windows
 * refuse `window.open`, so a window of its own never appeared and the button
 * did nothing. The frame holds only what belongs on the paper and loads
 * nothing — its own policy forbids every request, so no translated text can
 * make it fetch anything.
 */

export interface RecoverySheetText {
  title: string
  explanation: string
  code: string
  /** "Issued <date>", already filled in. */
  issued: string
}

/** What the sheet may load: nothing but its own inline styles. */
export const SHEET_POLICY = "default-src 'none'; style-src 'unsafe-inline'"

/** How long the print dialog has to start opening before it counts as failed. */
export const PRINT_START_TIMEOUT_MS = 2000

export function recoverySheetHtml(text: RecoverySheetText): string {
  const title = escapeHtml(text.title)
  return `<!doctype html>
<html>
  <head>
    <meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="${escapeHtml(SHEET_POLICY)}">
    <title>${title}</title>
  </head>
  <body style="font-family: system-ui, sans-serif; padding: 48px; color: #111;">
    <h1 style="font-size: 18px; margin: 0 0 8px;">${title}</h1>
    <p style="font-size: 13px; color: #444; margin: 0 0 24px; max-width: 46em;">${escapeHtml(text.explanation)}</p>
    <pre style="font-size: 20px; letter-spacing: 1px; font-family: ui-monospace, monospace; margin: 0 0 24px; white-space: pre-wrap; word-break: break-all;">${escapeHtml(text.code)}</pre>
    <p style="font-size: 12px; color: #666;">${escapeHtml(text.issued)}</p>
  </body>
</html>`
}

/** The part of the frame's window that printing needs. */
export interface PrintWindow {
  print(): void
  addEventListener(type: 'beforeprint' | 'afterprint', listener: () => void): void
}

/** Runs `run` after `ms` and returns what cancels it. */
export type Schedule = (ms: number, run: () => void) => () => void

/**
 * Asks `sheet` to print. `onDone` runs once the dialog has closed; `onFailed`
 * when `print()` throws or no dialog starts opening within the timeout — a
 * webview that does not print returns from `print()` without a word. Returns
 * what stops the wait, for when the sheet goes away first.
 */
export function watchPrint(
  sheet: PrintWindow,
  handlers: { onDone: () => void; onFailed: (error: unknown) => void },
  schedule: Schedule,
  timeoutMs: number = PRINT_START_TIMEOUT_MS,
): () => void {
  let started = false
  sheet.addEventListener('beforeprint', () => {
    started = true
  })
  sheet.addEventListener('afterprint', handlers.onDone)

  try {
    sheet.print()
  } catch (error) {
    handlers.onFailed(error)
    return () => {}
  }
  // WebView2 holds `print()` until the dialog closes, so by now it has started.
  if (started) return () => {}

  return schedule(timeoutMs, () => {
    if (!started) handlers.onFailed(new Error('The print dialog did not open'))
  })
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

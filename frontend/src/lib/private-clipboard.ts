/**
 * Copying text so that Windows does not keep it.
 *
 * `navigator.clipboard.writeText` hands the text to clipboard history (Win+V)
 * and, if the owner turned it on, to the cloud clipboard — a transcript or a
 * recovery code copied once would outlive the paste, outside the archive. The
 * backend writes the same text with the formats that opt it out of both
 * (`clipboard.rs`).
 *
 * Only where the backend has no such write (not Windows) does this fall back to
 * an ordinary copy. A failure on Windows is reported, never quietly retried the
 * ordinary way: that would be the very copy this exists to avoid.
 */

import { invoke } from '@tauri-apps/api/core'

/** The backend's answer on a platform with no private write. */
export const UNSUPPORTED = 'unsupported'

type Write = (text: string) => Promise<unknown>

export async function copyTextWith(
  text: string,
  writePrivately: Write,
  writeOrdinarily: Write,
): Promise<void> {
  try {
    await writePrivately(text)
  } catch (error) {
    if (error === UNSUPPORTED) {
      await writeOrdinarily(text)
      return
    }
    throw error
  }
}

/** Puts `text` on the clipboard, out of clipboard history and cloud sync. */
export function copyText(text: string): Promise<void> {
  return copyTextWith(
    text,
    (value) => invoke('copy_text_privately', { text: value }),
    (value) => navigator.clipboard.writeText(value),
  )
}

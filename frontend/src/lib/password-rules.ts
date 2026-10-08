/**
 * The rules a new password has to meet, checked while it is typed.
 *
 * Every form that sets a password — turning protection on, changing it, and
 * resetting it with the recovery code — asks for it twice and uses this, so a
 * typo cannot lock the archive behind a password nobody knows. The backend
 * checks the length again and has the last word.
 */

/**
 * Shortest password the backend accepts. Mirrors `MIN_PASSWORD_LEN` in
 * `src-tauri/src/security/keystore.rs`; change both together.
 */
export const MIN_PASSWORD_LENGTH = 8

export interface NewPasswordCheck {
  /** Something is typed, but fewer characters than the minimum. */
  tooShort: boolean
  /** The repeat differs from the password in a way more typing cannot fix. */
  mismatch: boolean
  /** Both fields are filled in and every rule is met. */
  ok: boolean
}

/**
 * Counts characters the way the backend does (Rust `chars()`), so a password
 * with emoji or other characters outside the BMP is measured the same on both
 * sides.
 */
function length(text: string): number {
  return Array.from(text).length
}

export function checkNewPassword(password: string, repeat: string): NewPasswordCheck {
  const tooShort = password.length > 0 && length(password) < MIN_PASSWORD_LENGTH
  // While the repeat is still the beginning of the password, the owner is most
  // likely still typing it: saying "do not match" after the first character
  // would only be noise.
  const mismatch = repeat.length > 0 && repeat !== password && !password.startsWith(repeat)
  const ok = password.length > 0 && !tooShort && repeat === password
  return { tooShort, mismatch, ok }
}

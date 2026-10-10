/**
 * Whether the window should remind the owner that the recovery code they just
 * used still opens the archive.
 *
 * Getting in with the code does not retire it — the backend keeps it working so
 * the printout is never dead before a new one has been written down. But the
 * sheet has just been out of the drawer, and someone may have seen it. So after
 * such a way in the window says so once, offers to issue a new code, and keeps
 * a quieter line on the security screen until a new code exists or the old one
 * is gone.
 *
 * Nothing here holds the code or a password: only what happened, in memory, for
 * this run of the app.
 */

export interface RecoveryNotice {
  /** The current code was used to get in during this run. */
  codeUsed: boolean
  /** The one-time reminder is on screen. */
  open: boolean
}

export type RecoveryEvent =
  /** The archive was opened with the recovery code (with or without a reset). */
  | 'unlockedWithCode'
  /** The owner closed the reminder without issuing a new code. */
  | 'dismissed'
  /** A new code replaced the used one. */
  | 'codeReissued'
  /** The code was deleted; there is nothing left to warn about. */
  | 'codeRemoved'

export const NO_RECOVERY_NOTICE: RecoveryNotice = { codeUsed: false, open: false }

export function nextRecoveryNotice(
  notice: RecoveryNotice,
  event: RecoveryEvent,
): RecoveryNotice {
  switch (event) {
    case 'unlockedWithCode':
      return { codeUsed: true, open: true }
    case 'dismissed':
      return { ...notice, open: false }
    case 'codeReissued':
    case 'codeRemoved':
      return NO_RECOVERY_NOTICE
  }
}

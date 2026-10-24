/**
 * Whether the app behind the lock screen is showing.
 *
 * The lock screen replaces the app while the archive is locked, and an empty
 * ground stands in while the state is still being read. Anything that reaches
 * the window from outside — a file dropped onto it — must be ignored in both
 * cases, or it surfaces over the lock screen or after the owner gets in.
 */

export interface ArchiveGateState {
  loading: boolean
  status: { state: string } | null
}

export function archiveIsOpen({ loading, status }: ArchiveGateState): boolean {
  return !loading && status?.state !== 'locked'
}

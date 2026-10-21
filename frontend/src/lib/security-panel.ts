/**
 * Which action on the security panel is running, and where its outcome goes.
 *
 * Every action keeps its own place: the button that was pressed says it is
 * working while the others only go quiet, and an error or a notice shows next
 * to the field or button it is about rather than at the bottom of the panel.
 * A refusal for too many attempts also starts a wait that every password field
 * on the panel honours, since the backend counts attempts for the archive as a
 * whole.
 */

export type SecurityAction =
  | 'setup'
  | 'lock'
  | 'encryptRecordings'
  | 'encryptFields'
  | 'deleteBackup'
  | 'changePassword'
  | 'recoveryIssue'
  | 'recoveryRemove'
  | 'quick'
  | 'autoLock'
  | 'keyBackup'
  | 'disable'

export interface SecurityOutcome {
  /** The action whose place shows it. */
  place: SecurityAction
  /** `wait` is a refusal for too many attempts: its text counts down, so the panel renders it. */
  kind: 'error' | 'notice' | 'wait'
  text: string
}

export interface SecurityPanelState {
  running: SecurityAction | null
  outcome: SecurityOutcome | null
  /** When the backend lets the next password attempt through, in ms; null when it does now. */
  waitUntil: number | null
}

export const IDLE_PANEL: SecurityPanelState = { running: null, outcome: null, waitUntil: null }

export type SecurityPanelEvent =
  | { type: 'start'; action: SecurityAction }
  | { type: 'succeed'; notice?: string; place?: SecurityAction }
  | { type: 'fail'; text: string; waitSeconds?: number; now: number }
  | { type: 'waitOver' }
  | { type: 'dismiss' }

export function securityPanelReducer(
  state: SecurityPanelState,
  event: SecurityPanelEvent,
): SecurityPanelState {
  switch (event.type) {
    case 'start':
      // One action at a time: the backend holds one key and one attempt counter.
      if (state.running) return state
      return { ...state, running: event.action, outcome: null }
    case 'succeed': {
      if (!state.running) return state
      const place = event.place ?? state.running
      return {
        ...state,
        running: null,
        outcome: event.notice ? { place, kind: 'notice', text: event.notice } : null,
      }
    }
    case 'fail': {
      if (!state.running) return state
      if (event.waitSeconds && event.waitSeconds > 0) {
        return {
          running: null,
          outcome: { place: state.running, kind: 'wait', text: event.text },
          waitUntil: event.now + event.waitSeconds * 1000,
        }
      }
      return {
        ...state,
        running: null,
        outcome: { place: state.running, kind: 'error', text: event.text },
      }
    }
    case 'waitOver':
      // The refusal is over, so its message goes with it; anything else stays.
      return {
        ...state,
        waitUntil: null,
        outcome: state.outcome?.kind === 'wait' ? null : state.outcome,
      }
    case 'dismiss':
      return { ...state, outcome: null }
  }
}

/** Whole seconds left to wait, rounded up so the count never shows 0 while it still refuses. */
export function secondsLeft(waitUntil: number | null, now: number): number {
  if (waitUntil === null) return 0
  return Math.max(0, Math.ceil((waitUntil - now) / 1000))
}

/** The outcome to show at one place, if it belongs there. */
export function outcomeAt(
  state: SecurityPanelState,
  place: SecurityAction,
): SecurityOutcome | null {
  return state.outcome?.place === place ? state.outcome : null
}

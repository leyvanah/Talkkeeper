'use client'

/**
 * The lock, as the window sees it.
 *
 * Holds no key and no password: it asks the backend what state the archive is
 * in and passes typed secrets straight through. Every secret is dropped from
 * component state the moment its command returns.
 */

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

/** What the window should be showing. */
export type LockState = 'unconfigured' | 'locked' | 'unlocked'

export interface SecurityStatus {
  state: LockState
  hasRecovery: boolean
  waitSeconds: number
  autoLockMinutes: number | null
  failedAttempts: number
  /** Whether this machine could show a Windows Hello prompt at all. */
  quickAvailable: boolean
  /** Whether the owner turned quick unlock on. Off unless they did. */
  quickEnabled: boolean
}

/**
 * How the recordings on disk actually stand, which is not always what the
 * password implies: a conversion interrupted halfway, or a folder restored from
 * a backup made before the password existed, leaves some of them plaintext.
 */
export interface RecordingEncryption {
  encrypted: number
  plaintext: number
}

/**
 * The same question about the database: how many of the values that hold what
 * was said are sealed, and how many are still readable. A `NULL` counts as
 * neither, so the two numbers are of values that exist.
 */
export interface FieldEncryption {
  sealed: number
  plaintext: number
  /** Whether the copy taken before the first encryption is still on disk. */
  plaintextBackup: boolean
}

/**
 * A failure the interface reacts to by code. `message` is English and belongs in
 * the console, never on screen — the lock screen renders its own text.
 */
export interface SecurityError {
  code: string
  message: string
  waitSeconds?: number
  minimum?: number
}

/** Turns whatever came back from `invoke` into something with a `code`. */
export function asSecurityError(error: unknown): SecurityError {
  if (error && typeof error === 'object' && 'code' in error) {
    return error as SecurityError
  }
  return { code: 'unknown', message: String(error) }
}

interface SecurityContextValue {
  status: SecurityStatus | null
  /** True until the first status has been read; the window shows nothing yet. */
  loading: boolean
  refresh: () => Promise<SecurityStatus | null>
  unlock: (password: string) => Promise<void>
  unlockWithRecovery: (code: string) => Promise<void>
  lock: () => Promise<void>
  setup: (password: string, withRecovery: boolean) => Promise<string | null>
  changePassword: (current: string, next: string) => Promise<void>
  resetPassword: (code: string, next: string) => Promise<void>
  regenerateRecovery: (password: string) => Promise<string | null>
  removeRecovery: (password: string) => Promise<void>
  setAutoLock: (minutes: number | null) => Promise<void>
  disable: (password: string) => Promise<void>
  /** Counts the recordings on disk by whether they are encrypted. */
  recordingEncryption: () => Promise<RecordingEncryption>
  /** Brings the plaintext ones under the key, and reports the new count. */
  encryptRecordings: () => Promise<RecordingEncryption>
  /** Counts the database values by whether they are sealed. */
  fieldEncryption: () => Promise<FieldEncryption>
  /** Seals the ones that are not, and reports the new count. */
  encryptFields: () => Promise<FieldEncryption>
  /** Deletes the unencrypted copy taken before the first encryption. */
  deletePlaintextBackup: () => Promise<FieldEncryption>
  /** Opens the archive with a Windows Hello prompt. */
  quickUnlock: (prompt: string) => Promise<void>
  /** Turns quick unlock on, proving the password first. */
  quickEnable: (password: string) => Promise<void>
  /** Turns quick unlock off, back to password only. */
  quickDisable: () => Promise<void>
}

const SecurityContext = createContext<SecurityContextValue | null>(null)

/** How often the window tells the backend the owner is still here. */
const TOUCH_INTERVAL_MS = 30_000

export function SecurityProvider({ children }: { children: React.ReactNode }) {
  const [status, setStatus] = useState<SecurityStatus | null>(null)
  const [loading, setLoading] = useState(true)

  const refresh = useCallback(async () => {
    try {
      const next = await invoke<SecurityStatus>('security_status')
      setStatus(next)
      return next
    } catch (error) {
      console.error('[Security] Could not read the lock state:', error)
      return null
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    refresh()
  }, [refresh])

  // The backend locks on its own after an idle timeout; the window finds out here.
  useEffect(() => {
    const unlisten = listen('archive-locked', () => {
      console.log('[Security] Archive locked after idle timeout')
      refresh()
    })
    return () => {
      unlisten.then((stop) => stop())
    }
  }, [refresh])

  // Presence, reported at a coarse interval rather than per keystroke: the idle
  // timeout is measured in minutes, so a half-minute of granularity is plenty
  // and costs one call instead of thousands.
  const activeSinceLastTick = useRef(false)
  useEffect(() => {
    if (status?.state !== 'unlocked') return

    const noteActivity = () => {
      activeSinceLastTick.current = true
    }
    const events: (keyof WindowEventMap)[] = ['pointerdown', 'keydown', 'wheel', 'focus']
    events.forEach((event) => window.addEventListener(event, noteActivity, { passive: true }))

    const timer = window.setInterval(() => {
      if (!activeSinceLastTick.current) return
      activeSinceLastTick.current = false
      invoke('security_touch').catch(() => {})
    }, TOUCH_INTERVAL_MS)

    return () => {
      events.forEach((event) => window.removeEventListener(event, noteActivity))
      window.clearInterval(timer)
    }
  }, [status?.state])

  const value = useMemo<SecurityContextValue>(() => {
    /** Runs a command, then re-reads the state the command may have changed. */
    const run = async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
      try {
        return await invoke<T>(command, args)
      } finally {
        await refresh()
      }
    }

    return {
      status,
      loading,
      refresh,
      unlock: (password) => run<void>('security_unlock', { password }),
      unlockWithRecovery: (code) => run<void>('security_unlock_with_recovery', { code }),
      lock: () => run<void>('security_lock'),
      setup: async (password, withRecovery) => {
        const response = await run<{ recoveryCode: string | null }>('security_setup', {
          password,
          withRecovery,
        })
        return response.recoveryCode
      },
      changePassword: (currentPassword, newPassword) =>
        run<void>('security_change_password', { currentPassword, newPassword }),
      resetPassword: (recoveryCode, newPassword) =>
        run<void>('security_reset_password', { recoveryCode, newPassword }),
      regenerateRecovery: async (password) => {
        const response = await run<{ recoveryCode: string | null }>(
          'security_regenerate_recovery',
          { password },
        )
        return response.recoveryCode
      },
      removeRecovery: (password) => run<void>('security_remove_recovery', { password }),
      setAutoLock: (minutes) => run<void>('security_set_auto_lock', { minutes }),
      // Neither of these changes the lock state, so neither re-reads it.
      recordingEncryption: () => invoke<RecordingEncryption>('security_recording_encryption'),
      encryptRecordings: () => invoke<RecordingEncryption>('security_encrypt_recordings'),
      fieldEncryption: () => invoke<FieldEncryption>('security_field_encryption'),
      encryptFields: () => invoke<FieldEncryption>('security_encrypt_fields'),
      deletePlaintextBackup: () =>
        invoke<FieldEncryption>('security_delete_plaintext_backup'),
      disable: (password) => run<void>('security_disable', { password }),
      quickUnlock: (prompt) => run<void>('security_quick_unlock', { prompt }),
      quickEnable: (password) => run<void>('security_quick_enable', { password }),
      quickDisable: () => run<void>('security_quick_disable'),
    }
  }, [status, loading, refresh])

  return <SecurityContext.Provider value={value}>{children}</SecurityContext.Provider>
}

export function useSecurity(): SecurityContextValue {
  const context = useContext(SecurityContext)
  if (!context) {
    throw new Error('useSecurity must be used inside a SecurityProvider')
  }
  return context
}

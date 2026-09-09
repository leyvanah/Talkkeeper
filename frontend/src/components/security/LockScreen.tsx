'use client'

/**
 * The screen the archive opens on when it is locked.
 *
 * Covers everything: it is rendered instead of the app, not over it, so nothing
 * behind it has mounted, fetched or cached anything.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslations } from 'next-intl'
import { Lock, KeyRound, ArrowLeft, Eye, EyeOff, ScanFace } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { asSecurityError, useSecurity } from '@/contexts/SecurityContext'
import { RecoveryCodeInput } from './RecoveryCodeInput'

/** Which of the two ways in the owner is currently using. */
type Mode = 'password' | 'recovery'

export function LockScreen() {
  const t = useTranslations('security')
  const { status, unlock, resetPassword, quickUnlock } = useSecurity()

  const [mode, setMode] = useState<Mode>('password')
  const [password, setPassword] = useState('')
  const [reveal, setReveal] = useState(false)
  const [recoveryCode, setRecoveryCode] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [waitSeconds, setWaitSeconds] = useState(status?.waitSeconds ?? 0)

  const passwordField = useRef<HTMLInputElement>(null)

  useEffect(() => {
    passwordField.current?.focus()
  }, [mode])

  // The wait is enforced by the backend; this only shows it ticking down so the
  // owner is not staring at a button that silently refuses to work.
  useEffect(() => {
    setWaitSeconds(status?.waitSeconds ?? 0)
  }, [status?.waitSeconds])

  useEffect(() => {
    if (waitSeconds <= 0) return
    const timer = window.setInterval(() => {
      setWaitSeconds((seconds) => Math.max(0, seconds - 1))
    }, 1000)
    return () => window.clearInterval(timer)
  }, [waitSeconds])

  /** Maps a backend error code onto text in the owner's language. */
  const describe = useCallback(
    (error: unknown): string => {
      const failure = asSecurityError(error)
      switch (failure.code) {
        case 'wrongPassword':
          return t('errorWrongPassword')
        case 'wrongRecoveryCode':
          return t('errorWrongRecoveryCode')
        case 'recoveryTypo':
          return t('errorRecoveryTypo')
        case 'recoveryWrongLength':
          return t('errorRecoveryWrongLength')
        case 'recoveryBadCharacter':
          return t('errorRecoveryBadCharacter')
        case 'tooManyAttempts':
          setWaitSeconds(failure.waitSeconds ?? 0)
          return t('errorTooManyAttempts', { seconds: failure.waitSeconds ?? 0 })
        case 'passwordTooShort':
          return t('errorPasswordTooShort', { minimum: failure.minimum ?? 8 })
        case 'noRecoveryCode':
          return t('errorNoRecoveryCode')
        case 'keystoreCorrupt':
          return t('errorKeystoreCorrupt')
        case 'quickDeclined':
          return t('errorQuickDeclined')
        case 'quickUnavailable':
          return t('errorQuickUnavailable')
        case 'quickKeyUnusable':
          return t('errorQuickKeyUnusable')
        default:
          console.error('[LockScreen] Unlock failed:', failure)
          return t('errorUnknown')
      }
    },
    [t],
  )

  const submitPassword = async (event: React.FormEvent) => {
    event.preventDefault()
    if (busy || waitSeconds > 0 || password.length === 0) return

    setBusy(true)
    setError(null)
    try {
      await unlock(password)
      setPassword('')
    } catch (failure) {
      setError(describe(failure))
      setPassword('')
      passwordField.current?.focus()
    } finally {
      setBusy(false)
    }
  }

  const submitRecovery = async (event: React.FormEvent) => {
    event.preventDefault()
    if (busy || waitSeconds > 0) return

    setBusy(true)
    setError(null)
    try {
      // Coming in on the recovery code always sets a new password: an owner who
      // needed the sheet of paper does not remember the old one.
      await resetPassword(recoveryCode, newPassword)
      setRecoveryCode('')
      setNewPassword('')
    } catch (failure) {
      setError(describe(failure))
    } finally {
      setBusy(false)
    }
  }

  const blocked = waitSeconds > 0

  return (
    <div className="flex h-screen items-center justify-center bg-[var(--af-bg)] px-6">
      <div className="w-full max-w-sm">
        <div className="mb-8 flex flex-col items-center text-center">
          <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-[var(--af-panel)] ring-1 ring-[var(--af-border)]">
            {mode === 'password' ? (
              <Lock className="h-6 w-6 text-[var(--af-text-2)]" />
            ) : (
              <KeyRound className="h-6 w-6 text-[var(--af-text-2)]" />
            )}
          </div>
          <h1 className="text-xl font-semibold text-[var(--af-text)]">
            {mode === 'password' ? t('lockTitle') : t('recoveryUnlockTitle')}
          </h1>
          <p className="mt-2 text-sm text-[var(--af-text-2)]">
            {mode === 'password' ? t('lockSubtitle') : t('recoveryUnlockSubtitle')}
          </p>
        </div>

        {mode === 'password' ? (
          <form onSubmit={submitPassword} className="space-y-4">
            <div className="relative">
              <Input
                ref={passwordField}
                type={reveal ? 'text' : 'password'}
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                placeholder={t('passwordPlaceholder')}
                autoComplete="current-password"
                disabled={busy || blocked}
                aria-label={t('passwordPlaceholder')}
                className="pr-10"
              />
              <button
                type="button"
                onClick={() => setReveal((shown) => !shown)}
                className="absolute inset-y-0 right-0 flex w-10 items-center justify-center text-[var(--af-text-2)] hover:text-[var(--af-text)]"
                aria-label={reveal ? t('hidePassword') : t('showPassword')}
                tabIndex={-1}
              >
                {reveal ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
              </button>
            </div>

            <Button type="submit" className="w-full" disabled={busy || blocked || !password}>
              {busy ? t('unlocking') : t('unlock')}
            </Button>

            {/* Offered only where the owner turned it on. The password field
                stays above it: this is the shortcut, not the way in. */}
            {status?.quickEnabled && (
              <Button
                type="button"
                variant="outline"
                className="w-full"
                disabled={busy || blocked}
                onClick={async () => {
                  setBusy(true)
                  setError(null)
                  try {
                    await quickUnlock(t('quickPrompt'))
                  } catch (failure) {
                    setError(describe(failure))
                  } finally {
                    setBusy(false)
                  }
                }}
              >
                <ScanFace className="mr-2 h-4 w-4" />
                {t('quickUnlockAction')}
              </Button>
            )}
          </form>
        ) : (
          <form onSubmit={submitRecovery} className="space-y-4">
            <RecoveryCodeInput
              value={recoveryCode}
              onChange={setRecoveryCode}
              disabled={busy || blocked}
            />
            <Input
              type="password"
              value={newPassword}
              onChange={(event) => setNewPassword(event.target.value)}
              placeholder={t('newPasswordPlaceholder')}
              autoComplete="new-password"
              disabled={busy || blocked}
              aria-label={t('newPasswordPlaceholder')}
            />
            <Button
              type="submit"
              className="w-full"
              disabled={busy || blocked || !recoveryCode || !newPassword}
            >
              {busy ? t('unlocking') : t('recoveryUnlockAction')}
            </Button>
          </form>
        )}

        {blocked && (
          <p className="mt-4 text-center text-sm text-amber-500">
            {t('waitBeforeNextAttempt', { seconds: waitSeconds })}
          </p>
        )}

        {error && !blocked && (
          <p role="alert" className="mt-4 text-center text-sm text-red-500">
            {error}
          </p>
        )}

        <div className="mt-6 text-center">
          {mode === 'password' ? (
            status?.hasRecovery ? (
              <button
                type="button"
                onClick={() => {
                  setMode('recovery')
                  setError(null)
                }}
                className="text-sm text-[var(--af-text-2)] underline-offset-4 hover:text-[var(--af-text)] hover:underline"
              >
                {t('forgotPassword')}
              </button>
            ) : (
              <p className="text-xs text-[var(--af-text-2)]">{t('noRecoveryHint')}</p>
            )
          ) : (
            <button
              type="button"
              onClick={() => {
                setMode('password')
                setError(null)
              }}
              className="inline-flex items-center gap-1.5 text-sm text-[var(--af-text-2)] underline-offset-4 hover:text-[var(--af-text)] hover:underline"
            >
              <ArrowLeft className="h-3.5 w-3.5" />
              {t('backToPassword')}
            </button>
          )}
        </div>
      </div>
    </div>
  )
}

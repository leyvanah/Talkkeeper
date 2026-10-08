'use client'

/**
 * Shown once, right after the archive was opened with the recovery code.
 *
 * The code keeps working after such a way in — on purpose, so the printout is
 * never dead before a new one exists — but the sheet has just been out of the
 * drawer. This says so and offers to issue a new code on the spot. Issuing one
 * asks for the password again: it is how the backend refuses an unattended
 * window, and here it also proves the password just set was typed as meant.
 */

import { useState } from 'react'
import { useTranslations } from 'next-intl'
import { KeyRound } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { asSecurityError, useSecurity } from '@/contexts/SecurityContext'
import { RecoveryCodeCard } from './RecoveryCodeCard'

export function RecoveryUsedDialog() {
  const t = useTranslations('security')
  const { status, recoveryNotice, dismissRecoveryNotice, regenerateRecovery } = useSecurity()

  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // The new code, once issued. Kept here rather than read from the notice,
  // which closes as soon as the code is replaced.
  const [freshCode, setFreshCode] = useState<string | null>(null)

  const open = freshCode !== null || (recoveryNotice.open && status?.state === 'unlocked')
  if (!open) return null

  const close = () => {
    setPassword('')
    setError(null)
    setFreshCode(null)
    dismissRecoveryNotice()
  }

  const issue = async (event: React.FormEvent) => {
    event.preventDefault()
    if (busy || !password) return

    setBusy(true)
    setError(null)
    try {
      const code = await regenerateRecovery(password)
      setPassword('')
      if (code) setFreshCode(code)
    } catch (failure) {
      const reason = asSecurityError(failure)
      if (reason.code === 'wrongPassword') {
        setError(t('errorWrongPassword'))
      } else if (reason.code === 'tooManyAttempts') {
        setError(t('errorTooManyAttempts', { seconds: reason.waitSeconds ?? 0 }))
      } else {
        console.error('[Security] Could not issue a new recovery code:', reason)
        setError(t('errorUnknown'))
      }
      setPassword('')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(next) => {
        // The new code is shown once; it goes only through its own button.
        if (!next && !freshCode && !busy) close()
      }}
    >
      <DialogContent
        showCloseButton={false}
        onPointerDownOutside={(event) => event.preventDefault()}
        className="max-h-[calc(100vh-2rem)] max-w-[480px] overflow-y-auto border-[var(--af-border)] bg-[var(--af-panel)]"
      >
        <DialogHeader className="text-left">
          <div className="mb-2 flex h-10 w-10 items-center justify-center rounded-xl bg-amber-500/10 text-amber-500">
            <KeyRound className="h-5 w-5" />
          </div>
          <DialogTitle className="text-[var(--af-text)]">
            {freshCode ? t('recoveryCodeTitle') : t('recoveryUsedTitle')}
          </DialogTitle>
          {!freshCode && (
            <DialogDescription className="text-[var(--af-text-2)]">
              {t('recoveryUsedBody')}
            </DialogDescription>
          )}
        </DialogHeader>

        {freshCode ? (
          <RecoveryCodeCard code={freshCode} onConfirmed={close} />
        ) : (
          <form onSubmit={issue} className="space-y-3">
            <Input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder={t('recoveryUsedPasswordPlaceholder')}
              autoComplete="current-password"
              disabled={busy}
              aria-label={t('recoveryUsedPasswordPlaceholder')}
              autoFocus
            />
            {error && (
              <p role="alert" className="text-sm text-red-500">
                {error}
              </p>
            )}
            <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
              <Button type="button" variant="ghost" onClick={close} disabled={busy}>
                {t('recoveryUsedLater')}
              </Button>
              <Button type="submit" disabled={busy || !password}>
                {busy ? t('recoveryUsedIssuing') : t('recoveryCodeReissue')}
              </Button>
            </div>
          </form>
        )}
      </DialogContent>
    </Dialog>
  )
}

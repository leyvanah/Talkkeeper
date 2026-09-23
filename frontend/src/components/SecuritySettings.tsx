'use client'

/**
 * The lock, in Settings: turning it on, changing the password, reissuing the
 * recovery code, and how long the archive stays open unattended.
 */

import { useCallback, useEffect, useState } from 'react'
import { useTranslations } from 'next-intl'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { save } from '@tauri-apps/plugin-dialog'
import {
  Database,
  FileLock2,
  KeyRound,
  Lock,
  ScanFace,
  ShieldCheck,
  Timer,
  TriangleAlert,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import {
  asSecurityError,
  useSecurity,
  type FieldEncryption,
  type RecordingEncryption,
} from '@/contexts/SecurityContext'
import { RecoveryCodeCard } from '@/components/security/RecoveryCodeCard'
import { CloudPrivacyCard } from '@/components/security/CloudPrivacyCard'
import { LocalOnlyCard } from '@/components/security/LocalOnlyCard'

/** Idle timeouts offered, in minutes. `0` stands for "never". */
const AUTO_LOCK_CHOICES = [0, 5, 15, 30, 60] as const

export function SecuritySettings() {
  const t = useTranslations('security')
  const {
    status,
    setup,
    changePassword,
    regenerateRecovery,
    removeRecovery,
    setAutoLock,
    disable,
    lock,
    quickEnable,
    quickDisable,
    recordingEncryption,
    encryptRecordings,
    fieldEncryption,
    encryptFields,
    deletePlaintextBackup,
  } = useSecurity()

  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  /** Set while a freshly issued recovery code is being shown. */
  const [freshCode, setFreshCode] = useState<string | null>(null)
  /** How the recordings on disk stand; null until counted. */
  const [recordings, setRecordings] = useState<RecordingEncryption | null>(null)
  /** How the database columns stand; null until counted. */
  const [dbFields, setDbFields] = useState<FieldEncryption | null>(null)
  /** Set while the owner is being asked to confirm deleting the plaintext copy. */
  const [confirmingBackupDelete, setConfirmingBackupDelete] = useState(false)
  /** Progress of a conversion, while one is running. */
  const [converting, setConverting] = useState<{ done: number; total: number } | null>(null)

  // Setting protection up.
  const [newPassword, setNewPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [wantRecovery, setWantRecovery] = useState(true)

  // Changing an existing password.
  const [currentPassword, setCurrentPassword] = useState('')
  const [replacementPassword, setReplacementPassword] = useState('')

  // Proving intent for the recovery code, and for removing protection.
  const [confirmingPassword, setConfirmingPassword] = useState('')

  // Proving intent for quick unlock. Its own value, so the field can sit in its
  // own section instead of being shared across the panel.
  const [quickPassword, setQuickPassword] = useState('')

  // Counting walks the recording folders, so it happens once the archive is
  // open rather than on every render of the panel.
  useEffect(() => {
    if (status?.state !== 'unlocked') return
    let current = true
    recordingEncryption()
      .then((counts) => {
        if (current) setRecordings(counts)
      })
      .catch((failure) => console.error('[SecuritySettings] Could not count recordings:', failure))
    return () => {
      current = false
    }
  }, [status?.state, recordingEncryption])

  // The database count is a handful of queries rather than a walk of the disk,
  // but it is asked at the same moment and for the same reason: a partly
  // converted archive is a real state and should be visible.
  useEffect(() => {
    if (status?.state !== 'unlocked') return
    let current = true
    fieldEncryption()
      .then((counts) => {
        if (current) setDbFields(counts)
      })
      .catch((failure) =>
        console.error('[SecuritySettings] Could not count database values:', failure),
      )
    return () => {
      current = false
    }
  }, [status?.state, fieldEncryption])

  // A conversion reports itself as it goes: an archive of a year's sessions is
  // gigabytes, and a panel showing nothing is a panel the owner force-quits.
  useEffect(() => {
    const unlisten = listen<{ done: number; total: number }>('archive-conversion', (event) => {
      const { done, total } = event.payload
      setConverting(done >= total ? null : { done, total })
    })
    return () => {
      unlisten.then((stop) => stop())
    }
  }, [])

  const describe = useCallback(
    (failure: unknown): string => {
      const problem = asSecurityError(failure)
      switch (problem.code) {
        case 'wrongPassword':
          return t('errorWrongPassword')
        case 'passwordTooShort':
          return t('errorPasswordTooShort', { minimum: problem.minimum ?? 8 })
        case 'tooManyAttempts':
          return t('errorTooManyAttempts', { seconds: problem.waitSeconds ?? 0 })
        case 'recordingInProgress':
          return t('errorRecordingInProgress')
        case 'jobInProgress':
          return t('errorJobInProgress')
        case 'alreadyConfigured':
          return t('errorAlreadyConfigured')
        case 'quickDeclined':
          return t('errorQuickDeclined')
        case 'quickUnavailable':
          return t('errorQuickUnavailable')
        case 'quickKeyUnusable':
          return t('errorQuickKeyUnusable')
        case 'quickKeySetupFailed':
          return t('errorQuickKeySetupFailed')
        case 'decryptionIncomplete':
          return t('errorDecryptionIncomplete')
        case 'locked':
          return t('errorLocked')
        default:
          console.error('[SecuritySettings] Command failed:', problem)
          return t('errorUnknown')
      }
    },
    [t],
  )

  /** Runs an action, showing whichever of the two outcomes happened. */
  const attempt = async (action: () => Promise<void>, success?: string) => {
    setBusy(true)
    setError(null)
    setNotice(null)
    try {
      await action()
      if (success) setNotice(success)
    } catch (failure) {
      setError(describe(failure))
    } finally {
      setBusy(false)
    }
  }

  if (!status) {
    return <div className="p-6 text-sm text-[var(--af-text-2)]">{t('loading')}</div>
  }

  // A newly issued code takes over the panel: it exists in readable form only
  // here, and only until this is dismissed.
  if (freshCode) {
    return (
      <div className="mx-auto max-w-2xl rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
        <h3 className="mb-4 flex items-center gap-2 text-lg font-semibold text-gray-900">
          <KeyRound className="h-5 w-5 text-blue-500" />
          {t('recoveryCodeTitle')}
        </h3>
        <RecoveryCodeCard code={freshCode} onConfirmed={() => setFreshCode(null)} />
      </div>
    )
  }

  const configured = status.state !== 'unconfigured'

  return (
    <div className="space-y-6">
      <div className="rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
        <div className="flex items-start gap-3">
          <Lock className="mt-0.5 h-5 w-5 shrink-0 text-blue-500" />
          <div className="min-w-0 flex-1">
            <h3 className="text-lg font-semibold text-gray-900">{t('settingsTitle')}</h3>
            <p className="mt-1 text-sm text-gray-600">{t('settingsDescription')}</p>
          </div>
        </div>

        {/* Said plainly rather than implied: at this stage the lock guards the
            window, and the files on disk are not yet encrypted. */}
        <div className="mt-4 flex gap-3 rounded-lg border border-amber-500/40 bg-amber-500/10 p-4">
          <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-500" />
          <p className="text-sm text-[var(--af-text-2)]">{t('scopeNotice')}</p>
        </div>

        {!configured ? (
          <form
            className="mt-6 space-y-4"
            onSubmit={(event) => {
              event.preventDefault()
              if (newPassword !== confirmPassword) {
                setError(t('errorPasswordsDoNotMatch'))
                return
              }
              attempt(async () => {
                const code = await setup(newPassword, wantRecovery)
                setNewPassword('')
                setConfirmPassword('')
                if (code) setFreshCode(code)
              }, wantRecovery ? undefined : t('noticeProtectionOn'))
            }}
          >
            <Input
              type="password"
              value={newPassword}
              onChange={(event) => setNewPassword(event.target.value)}
              placeholder={t('newPasswordPlaceholder')}
              autoComplete="new-password"
              disabled={busy}
              aria-label={t('newPasswordPlaceholder')}
            />
            <Input
              type="password"
              value={confirmPassword}
              onChange={(event) => setConfirmPassword(event.target.value)}
              placeholder={t('confirmPasswordPlaceholder')}
              autoComplete="new-password"
              disabled={busy}
              aria-label={t('confirmPasswordPlaceholder')}
            />

            <label className="flex cursor-pointer items-start gap-2.5 text-sm text-gray-700">
              <input
                type="checkbox"
                checked={wantRecovery}
                onChange={(event) => setWantRecovery(event.target.checked)}
                className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--af-accent)]"
                disabled={busy}
              />
              <span>
                {t('wantRecoveryCode')}
                <span className="mt-1 block text-xs text-gray-500">
                  {wantRecovery ? t('wantRecoveryCodeHint') : t('noRecoveryCodeWarning')}
                </span>
              </span>
            </label>

            <Button type="submit" disabled={busy || !newPassword || !confirmPassword}>
              {busy ? t('working') : t('enableProtection')}
            </Button>
          </form>
        ) : (
          <div className="mt-6 space-y-6">
            <div className="flex items-center gap-2 text-sm text-emerald-500">
              <ShieldCheck className="h-4 w-4" />
              {t('protectionOn')}
            </div>

            {/* The recordings on disk, counted rather than assumed. A partly
                converted archive is a real state — an interrupted conversion, a
                folder restored from an older backup — and it should be visible
                rather than implied by the password being set. */}
            <div>
              <label className="mb-1.5 flex items-center gap-2 text-sm font-medium text-gray-900">
                <FileLock2 className="h-4 w-4 text-gray-500" />
                {t('recordingsTitle')}
              </label>
              <p className="mb-2 text-xs text-gray-500">
                {converting
                  ? t('recordingsConverting', { done: converting.done, total: converting.total })
                  : !recordings
                    ? t('recordingsCounting')
                    : recordings.encrypted + recordings.plaintext === 0
                      ? t('recordingsNone')
                      : recordings.plaintext === 0
                        ? t('recordingsAllEncrypted', { count: recordings.encrypted })
                        : t('recordingsPartly', {
                            encrypted: recordings.encrypted,
                            plaintext: recordings.plaintext,
                          })}
              </p>
              {recordings && recordings.plaintext > 0 && (
                <>
                  <Button
                    type="button"
                    variant="outline"
                    disabled={busy}
                    onClick={() =>
                      attempt(async () => {
                        setRecordings(await encryptRecordings())
                        setConverting(null)
                      }, t('noticeRecordingsEncrypted'))
                    }
                  >
                    {busy ? t('working') : t('encryptExisting')}
                  </Button>
                  <p className="mt-2 text-xs text-amber-500">{t('encryptExistingHint')}</p>
                </>
              )}
            </div>

            {/* The same, for the words rather than the audio: titles, names,
                transcripts and summaries live in the database, and an archive
                recorded before the password existed still holds them in the
                clear until this pass runs. */}
            <div>
              <label className="mb-1.5 flex items-center gap-2 text-sm font-medium text-gray-900">
                <Database className="h-4 w-4 text-gray-500" />
                {t('fieldsTitle')}
              </label>
              <p className="mb-2 text-xs text-gray-500">
                {!dbFields
                  ? t('fieldsCounting')
                  : dbFields.sealed + dbFields.plaintext === 0
                    ? t('fieldsNone')
                    : dbFields.plaintext === 0
                      ? t('fieldsAllSealed', { count: dbFields.sealed })
                      : t('fieldsPartly', {
                          sealed: dbFields.sealed,
                          plaintext: dbFields.plaintext,
                        })}
              </p>
              {dbFields && dbFields.plaintext > 0 && (
                <>
                  <Button
                    type="button"
                    variant="outline"
                    disabled={busy}
                    onClick={() =>
                      attempt(async () => {
                        setDbFields(await encryptFields())
                      }, t('noticeFieldsEncrypted'))
                    }
                  >
                    {busy ? t('working') : t('encryptFields')}
                  </Button>
                  <p className="mt-2 text-xs text-gray-500">{t('encryptFieldsHint')}</p>
                </>
              )}
              {/* The copy taken before the first encryption is the whole archive
                  in the clear. It stays only until the owner has seen the
                  encrypted one read back, and this is where they let it go. */}
              {dbFields?.plaintextBackup && (
                <div className="mt-3 rounded-md border border-amber-300 bg-amber-50 p-3">
                  <p className="text-xs text-amber-800">{t('plaintextBackupWarning')}</p>
                  {confirmingBackupDelete ? (
                    <div className="mt-2 flex flex-wrap items-center gap-2">
                      <span className="text-xs font-medium text-amber-900">
                        {t('plaintextBackupConfirm')}
                      </span>
                      <Button
                        type="button"
                        variant="destructive"
                        size="sm"
                        disabled={busy}
                        onClick={() =>
                          attempt(async () => {
                            setDbFields(await deletePlaintextBackup())
                            setConfirmingBackupDelete(false)
                          }, t('noticePlaintextBackupDeleted'))
                        }
                      >
                        {busy ? t('working') : t('plaintextBackupDeleteYes')}
                      </Button>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        disabled={busy}
                        onClick={() => setConfirmingBackupDelete(false)}
                      >
                        {t('plaintextBackupKeep')}
                      </Button>
                    </div>
                  ) : (
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      className="mt-2"
                      disabled={busy}
                      onClick={() => setConfirmingBackupDelete(true)}
                    >
                      {t('plaintextBackupDelete')}
                    </Button>
                  )}
                </div>
              )}
            </div>

            {/* Idle timeout. Never applies while a recording is running. */}
            <div>
              <label className="mb-1.5 flex items-center gap-2 text-sm font-medium text-gray-900">
                <Timer className="h-4 w-4 text-gray-500" />
                {t('autoLockLabel')}
              </label>
              <p className="mb-2 text-xs text-gray-500">{t('autoLockHint')}</p>
              <select
                value={status.autoLockMinutes ?? 0}
                disabled={busy}
                onChange={(event) => {
                  const minutes = Number(event.target.value)
                  attempt(() => setAutoLock(minutes === 0 ? null : minutes))
                }}
                className="rounded-md border border-gray-200 bg-white px-3 py-2 text-sm text-gray-900 focus:border-blue-400 focus:outline-none"
              >
                {AUTO_LOCK_CHOICES.map((minutes) => (
                  <option key={minutes} value={minutes}>
                    {minutes === 0 ? t('autoLockNever') : t('autoLockMinutes', { minutes })}
                  </option>
                ))}
              </select>
            </div>

            {/* Quick unlock. Off unless the owner turns it on, and one switch
                puts the archive back to password-only. */}
            {status.quickAvailable && (
              <div className="border-t border-gray-200 pt-5">
                <div className="flex items-start justify-between gap-4">
                  <div className="min-w-0">
                    <h4 className="flex items-center gap-2 text-sm font-medium text-gray-900">
                      <ScanFace className="h-4 w-4 text-gray-500" />
                      {t('quickTitle')}
                    </h4>
                    <p className="mt-1 text-xs text-gray-500">{t('quickDescription')}</p>
                  </div>
                  <Switch
                    checked={status.quickEnabled}
                    // Turning it off needs nothing; turning it on needs the
                    // password typed in this section's own field, below.
                    disabled={busy || (!status.quickEnabled && !quickPassword)}
                    onCheckedChange={(wanted) =>
                      attempt(async () => {
                        if (wanted) {
                          await quickEnable(quickPassword)
                          setQuickPassword('')
                        } else {
                          await quickDisable()
                        }
                      }, wanted ? t('noticeQuickOn') : t('noticeQuickOff'))
                    }
                  />
                </div>

                {/* Said plainly next to the switch, not buried in a document:
                    this door is weaker than the password. */}
                <p className="mt-3 rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-xs text-[var(--af-text-2)]">
                  {t('quickWarning')}
                </p>

                {/* Its own field rather than one borrowed from the recovery-code
                    section further down: adding a door should not send the owner
                    hunting for an input under an unrelated heading. */}
                {!status.quickEnabled && (
                  <div className="mt-3">
                    <label className="mb-1.5 block text-xs text-gray-500">
                      {t('quickNeedsPassword')}
                    </label>
                    <Input
                      type="password"
                      value={quickPassword}
                      onChange={(event) => setQuickPassword(event.target.value)}
                      placeholder={t('currentPasswordPlaceholder')}
                      autoComplete="current-password"
                      disabled={busy}
                      aria-label={t('currentPasswordPlaceholder')}
                      className="max-w-xs"
                    />
                  </div>
                )}
              </div>
            )}

            <div className="border-t border-gray-200 pt-5">
              <h4 className="mb-3 text-sm font-medium text-gray-900">{t('changePasswordTitle')}</h4>
              <form
                className="space-y-3"
                onSubmit={(event) => {
                  event.preventDefault()
                  attempt(async () => {
                    await changePassword(currentPassword, replacementPassword)
                    setCurrentPassword('')
                    setReplacementPassword('')
                  }, t('noticePasswordChanged'))
                }}
              >
                <Input
                  type="password"
                  value={currentPassword}
                  onChange={(event) => setCurrentPassword(event.target.value)}
                  placeholder={t('currentPasswordPlaceholder')}
                  autoComplete="current-password"
                  disabled={busy}
                  aria-label={t('currentPasswordPlaceholder')}
                />
                <Input
                  type="password"
                  value={replacementPassword}
                  onChange={(event) => setReplacementPassword(event.target.value)}
                  placeholder={t('newPasswordPlaceholder')}
                  autoComplete="new-password"
                  disabled={busy}
                  aria-label={t('newPasswordPlaceholder')}
                />
                <p className="text-xs text-gray-500">{t('changePasswordKeepsRecovery')}</p>
                <Button
                  type="submit"
                  variant="outline"
                  disabled={busy || !currentPassword || !replacementPassword}
                >
                  {t('changePasswordAction')}
                </Button>
              </form>
            </div>

            <div className="border-t border-gray-200 pt-5">
              <h4 className="mb-1 text-sm font-medium text-gray-900">{t('recoveryCodeTitle')}</h4>
              <p className="mb-3 text-xs text-gray-500">
                {status.hasRecovery ? t('recoveryCodeExists') : t('recoveryCodeMissing')}
              </p>
              <form
                className="space-y-3"
                onSubmit={(event) => {
                  event.preventDefault()
                  attempt(async () => {
                    const code = await regenerateRecovery(confirmingPassword)
                    setConfirmingPassword('')
                    if (code) setFreshCode(code)
                  })
                }}
              >
                <Input
                  type="password"
                  value={confirmingPassword}
                  onChange={(event) => setConfirmingPassword(event.target.value)}
                  placeholder={t('currentPasswordPlaceholder')}
                  autoComplete="current-password"
                  disabled={busy}
                  aria-label={t('currentPasswordPlaceholder')}
                />
                <div className="flex flex-wrap gap-2">
                  <Button type="submit" variant="outline" disabled={busy || !confirmingPassword}>
                    {status.hasRecovery ? t('recoveryCodeReissue') : t('recoveryCodeCreate')}
                  </Button>
                  {status.hasRecovery && (
                    <Button
                      type="button"
                      variant="ghost"
                      disabled={busy || !confirmingPassword}
                      onClick={() =>
                        attempt(async () => {
                          await removeRecovery(confirmingPassword)
                          setConfirmingPassword('')
                        }, t('noticeRecoveryRemoved'))
                      }
                    >
                      {t('recoveryCodeRemove')}
                    </Button>
                  )}
                </div>
              </form>
            </div>

            <CloudPrivacyCard />

            <div className="border-t border-gray-200 pt-5">
              <h4 className="mb-1 text-sm font-medium text-gray-900">{t('keyBackupTitle')}</h4>
              <p className="mb-3 text-xs text-gray-500">{t('keyBackupDescription')}</p>
              <Button
                type="button"
                variant="outline"
                disabled={busy}
                onClick={async () => {
                  // Asked for before the work starts, so that closing the file
                  // picker does not report a backup that was never written.
                  const target = await save({
                    defaultPath: 'talkkeeper-key-backup.json',
                    filters: [{ name: 'JSON', extensions: ['json'] }],
                  })
                  if (!target) return
                  attempt(
                    () => invoke('security_export_key_backup', { path: target }),
                    t('noticeKeyBackupSaved'),
                  )
                }}
              >
                {t('keyBackupAction')}
              </Button>
            </div>

            <div className="flex flex-wrap gap-2 border-t border-gray-200 pt-5">
              <Button
                type="button"
                variant="outline"
                disabled={busy}
                onClick={() => attempt(() => lock())}
              >
                <Lock className="mr-2 h-4 w-4" />
                {t('lockNow')}
              </Button>
              <Button
                type="button"
                variant="ghost"
                className="text-red-500 hover:text-red-600"
                disabled={busy || !confirmingPassword}
                onClick={() =>
                  attempt(async () => {
                    await disable(confirmingPassword)
                    setConfirmingPassword('')
                  }, t('noticeProtectionOff'))
                }
              >
                {t('disableProtection')}
              </Button>
            </div>
            <p className="text-xs text-gray-500">{t('disableProtectionHint')}</p>
          </div>
        )}

        {error && (
          <p role="alert" className="mt-4 text-sm text-red-500">
            {error}
          </p>
        )}
        {notice && <p className="mt-4 text-sm text-emerald-500">{notice}</p>}
      </div>

      <LocalOnlyCard />
    </div>
  )
}

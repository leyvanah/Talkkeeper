'use client'

/**
 * The lock, in Settings: turning it on, encrypting what was there before,
 * the ways in (password, recovery code, Windows Hello, idle timeout), the key
 * backup and, last, turning it off. Network and cloud settings sit in their
 * own card below.
 *
 * Every action has its own password field, its own "Working…" and its own
 * place for the outcome (`lib/security-panel.ts`), so Enter in a field does
 * that field's action and nothing else, and an error shows where it happened.
 */

import { useCallback, useEffect, useReducer, useRef, useState } from 'react'
import { useTranslations } from 'next-intl'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { save } from '@tauri-apps/plugin-dialog'
import {
  Database,
  FileLock2,
  Globe,
  Info,
  KeyRound,
  Lock,
  ScanFace,
  ShieldCheck,
  ShieldOff,
  Timer,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { ConfirmDialog } from '@/components/ui/confirm-dialog'
import {
  asSecurityError,
  useSecurity,
  type FieldEncryption,
  type RecordingEncryption,
} from '@/contexts/SecurityContext'
import { RecoveryCodeCard } from '@/components/security/RecoveryCodeCard'
import { CloudPrivacyCard } from '@/components/security/CloudPrivacyCard'
import { LocalOnlyCard } from '@/components/security/LocalOnlyCard'
import { NewPasswordFields } from '@/components/security/NewPasswordFields'
import { PasswordInput } from '@/components/security/PasswordInput'
import { MIN_PASSWORD_LENGTH, checkNewPassword } from '@/lib/password-rules'
import {
  IDLE_PANEL,
  outcomeAt,
  secondsLeft,
  securityPanelReducer,
  type SecurityAction,
} from '@/lib/security-panel'

/** Idle timeouts offered, in minutes. `0` stands for "never". */
const AUTO_LOCK_CHOICES = [0, 5, 15, 30, 60] as const

/** A group of sections on the panel, under a small heading. */
function Group({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="space-y-5 border-t border-gray-200 pt-5">
      <h4 className="text-xs font-semibold uppercase tracking-wide text-gray-500">{title}</h4>
      {children}
    </section>
  )
}

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
    recoveryNotice,
  } = useSecurity()

  const [panel, dispatch] = useReducer(securityPanelReducer, IDLE_PANEL)
  /** Guards against a double click landing before the first one re-renders. */
  const runningRef = useRef(false)
  const [now, setNow] = useState(() => Date.now())

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
  const [replacementRepeat, setReplacementRepeat] = useState('')

  // One field per action that needs the password, so that Enter in it does
  // that action only: a shared field once reissued the recovery code for an
  // owner who meant to remove protection.
  const [recoveryPassword, setRecoveryPassword] = useState('')
  const [quickPassword, setQuickPassword] = useState('')
  const [removeCodePassword, setRemoveCodePassword] = useState('')
  const [disablePassword, setDisablePassword] = useState('')
  const [removingCode, setRemovingCode] = useState(false)
  const [disabling, setDisabling] = useState(false)

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

  // The wait is enforced by the backend; this only counts it down, so the
  // owner sees when trying again makes sense, as on the lock screen.
  useEffect(() => {
    if (panel.waitUntil === null) return
    const tick = () => {
      const moment = Date.now()
      setNow(moment)
      if (secondsLeft(panel.waitUntil, moment) === 0) dispatch({ type: 'waitOver' })
    }
    tick()
    const timer = window.setInterval(tick, 250)
    return () => window.clearInterval(timer)
  }, [panel.waitUntil])

  const describe = useCallback(
    (failure: unknown): string => {
      const problem = asSecurityError(failure)
      switch (problem.code) {
        case 'wrongPassword':
          return t('errorWrongPassword')
        case 'passwordTooShort':
          return t('errorPasswordTooShort', { minimum: problem.minimum ?? MIN_PASSWORD_LENGTH })
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
        case 'fieldVerificationFailed':
          return t('errorFieldVerificationFailed')
        case 'fieldVerificationIncomplete':
          return t('errorFieldVerificationIncomplete')
        case 'backupDeleteFailed':
          return t('errorBackupDeleteFailed')
        default:
          console.error('[SecuritySettings] Command failed:', problem)
          return t('errorUnknown')
      }
    },
    [t],
  )

  /**
   * Runs one action, marking it as the one in progress and putting its outcome
   * in its own place (or in `place`, when its section goes away on success).
   * Resolves to whether it worked.
   */
  const run = async (
    action: SecurityAction,
    work: () => Promise<void>,
    success?: string,
    place?: SecurityAction,
  ): Promise<boolean> => {
    if (runningRef.current) return false
    runningRef.current = true
    dispatch({ type: 'start', action })
    try {
      await work()
      dispatch({ type: 'succeed', notice: success, place })
      return true
    } catch (failure) {
      const problem = asSecurityError(failure)
      dispatch({
        type: 'fail',
        text: describe(failure),
        waitSeconds: problem.code === 'tooManyAttempts' ? problem.waitSeconds : undefined,
        now: Date.now(),
      })
      return false
    } finally {
      runningRef.current = false
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
  const setupCheck = checkNewPassword(newPassword, confirmPassword)
  const replacementCheck = checkNewPassword(replacementPassword, replacementRepeat)

  const busy = panel.running !== null
  const waiting = secondsLeft(panel.waitUntil, now) > 0
  /** Fields and buttons that send a password stay off while the backend makes us wait. */
  const passwordLocked = busy || waiting
  const label = (action: SecurityAction, text: string) =>
    panel.running === action ? t('working') : text

  /** The outcome of one action, shown where that action is. */
  const message = (place: SecurityAction, className = 'mt-2') => {
    const outcome = outcomeAt(panel, place)
    if (!outcome) return null
    if (outcome.kind === 'notice') {
      return <p className={`${className} text-sm text-emerald-500`}>{outcome.text}</p>
    }
    return (
      <p role="alert" className={`${className} text-sm text-red-500`}>
        {outcome.kind === 'wait'
          ? t('errorTooManyAttempts', { seconds: secondsLeft(panel.waitUntil, now) })
          : outcome.text}
      </p>
    )
  }

  const closeDialog = (place: SecurityAction, close: () => void) => {
    // An error belongs to the dialog and goes with it. A wait does not: it
    // keeps every password field off, so it moves under the section instead.
    if (outcomeAt(panel, place)?.kind === 'error') dispatch({ type: 'dismiss' })
    close()
  }

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

        {/* What the lock does and does not cover, for the state the archive is
            actually in: before a password, what one would add; after, what is
            encrypted and what stays visible. Information, not a warning. */}
        <div className="mt-4 flex gap-3 rounded-lg border border-[var(--af-border)] bg-[var(--af-panel)] p-4">
          <Info className="mt-0.5 h-4 w-4 shrink-0 text-[var(--af-text-2)]" />
          <p className="text-sm text-[var(--af-text-2)]">
            {configured ? t('scopeNotice') : t('scopeNoticeOff')}
          </p>
        </div>

        {!configured ? (
          <form
            className="mt-6 space-y-4"
            onSubmit={(event) => {
              event.preventDefault()
              if (!setupCheck.ok) return
              void run(
                'setup',
                async () => {
                  const code = await setup(newPassword, wantRecovery)
                  setNewPassword('')
                  setConfirmPassword('')
                  if (code) setFreshCode(code)
                },
                wantRecovery ? undefined : t('noticeProtectionOn'),
              )
            }}
          >
            <NewPasswordFields
              password={newPassword}
              repeat={confirmPassword}
              onPasswordChange={setNewPassword}
              onRepeatChange={setConfirmPassword}
              disabled={busy}
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

            <Button type="submit" disabled={busy || !setupCheck.ok}>
              {label('setup', t('enableProtection'))}
            </Button>
            {message('setup', 'mt-0')}
          </form>
        ) : (
          <div className="mt-6 space-y-6">
            {/* Status, and the way to lock the archive right now — at the
                top, away from removing protection. */}
            <div>
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex items-center gap-2 text-sm text-emerald-500">
                  <ShieldCheck className="h-4 w-4" />
                  {t('protectionOn')}
                </div>
                <div className="flex flex-col items-end gap-1">
                  <Button
                    type="button"
                    variant="outline"
                    disabled={busy}
                    onClick={() => void run('lock', () => lock())}
                  >
                    <Lock className="mr-2 h-4 w-4" />
                    {label('lock', t('lockNow'))}
                  </Button>
                  <p className="text-xs text-gray-500">{t('lockNowHint')}</p>
                </div>
              </div>
              {message('lock')}
              {message('setup')}
            </div>

            <Group title={t('sectionEncryption')}>
              {/* The recordings on disk, counted rather than assumed. A partly
                  converted archive is a real state — an interrupted conversion,
                  a folder restored from an older backup — and it should be
                  visible rather than implied by the password being set. */}
              <div>
                <h5 className="mb-1.5 flex items-center gap-2 text-sm font-medium text-gray-900">
                  <FileLock2 className="h-4 w-4 text-gray-500" />
                  {t('recordingsTitle')}
                </h5>
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
                        void run(
                          'encryptRecordings',
                          async () => {
                            setRecordings(await encryptRecordings())
                            setConverting(null)
                          },
                          t('noticeRecordingsEncrypted'),
                        )
                      }
                    >
                      {label('encryptRecordings', t('encryptExisting'))}
                    </Button>
                    <p className="mt-2 text-xs text-gray-500">{t('encryptExistingHint')}</p>
                  </>
                )}
                {message('encryptRecordings')}
              </div>

              {/* The same, for the words rather than the audio: titles, names,
                  transcripts and summaries live in the database, and an archive
                  recorded before the password existed still holds them in the
                  clear until this pass runs. */}
              <div>
                <h5 className="mb-1.5 flex items-center gap-2 text-sm font-medium text-gray-900">
                  <Database className="h-4 w-4 text-gray-500" />
                  {t('fieldsTitle')}
                </h5>
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
                        void run(
                          'encryptFields',
                          async () => {
                            try {
                              setDbFields(await encryptFields())
                            } catch (failure) {
                              // The pass can fail after it has committed, with
                              // the plaintext copy still on disk; re-read so the
                              // warning about it shows now rather than next visit.
                              const counts = await fieldEncryption().catch(() => null)
                              if (counts) setDbFields(counts)
                              throw failure
                            }
                          },
                          t('noticeFieldsEncrypted'),
                        )
                      }
                    >
                      {label('encryptFields', t('encryptFields'))}
                    </Button>
                    <p className="mt-2 text-xs text-gray-500">{t('encryptFieldsHint')}</p>
                  </>
                )}
                {message('encryptFields')}
                {/* The copy taken before the first encryption is the whole
                    archive in the clear. Encryption deletes it once every sealed
                    value opens; one still here was left by an older version, a
                    crash or a failed check, and this is where the owner lets it go. */}
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
                            void run(
                              'deleteBackup',
                              async () => {
                                setDbFields(await deletePlaintextBackup())
                                setConfirmingBackupDelete(false)
                              },
                              t('noticePlaintextBackupDeleted'),
                              'encryptFields',
                            )
                          }
                        >
                          {label('deleteBackup', t('plaintextBackupDeleteYes'))}
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
                    {message('deleteBackup')}
                  </div>
                )}
              </div>
            </Group>

            <Group title={t('sectionSignIn')}>
              <div>
                <h5 className="mb-3 text-sm font-medium text-gray-900">
                  {t('changePasswordTitle')}
                </h5>
                <form
                  className="space-y-3"
                  onSubmit={(event) => {
                    event.preventDefault()
                    if (passwordLocked || !currentPassword || !replacementCheck.ok) return
                    void run(
                      'changePassword',
                      async () => {
                        await changePassword(currentPassword, replacementPassword)
                        setCurrentPassword('')
                        setReplacementPassword('')
                        setReplacementRepeat('')
                      },
                      t('noticePasswordChanged'),
                    )
                  }}
                >
                  <PasswordInput
                    value={currentPassword}
                    onChange={setCurrentPassword}
                    placeholder={t('currentPasswordPlaceholder')}
                    autoComplete="current-password"
                    disabled={passwordLocked}
                  />
                  <NewPasswordFields
                    password={replacementPassword}
                    repeat={replacementRepeat}
                    onPasswordChange={setReplacementPassword}
                    onRepeatChange={setReplacementRepeat}
                    disabled={passwordLocked}
                  />
                  <p className="text-xs text-gray-500">{t('changePasswordKeepsRecovery')}</p>
                  <Button
                    type="submit"
                    variant="outline"
                    disabled={passwordLocked || !currentPassword || !replacementCheck.ok}
                  >
                    {label('changePassword', t('changePasswordAction'))}
                  </Button>
                  {message('changePassword', 'mt-0')}
                </form>
              </div>

              <div>
                <h5 className="mb-1 text-sm font-medium text-gray-900">{t('recoveryCodeTitle')}</h5>
                <p className="mb-3 text-xs text-gray-500">
                  {status.hasRecovery ? t('recoveryCodeExists') : t('recoveryCodeMissing')}
                </p>
                {status.hasRecovery && recoveryNotice.codeUsed && (
                  <p className="mb-3 text-xs text-amber-600">{t('recoveryCodeStillValid')}</p>
                )}
                {/* This field issues a code and nothing else; removing the code
                    asks for the password in its own confirmation. */}
                <form
                  className="space-y-3"
                  onSubmit={(event) => {
                    event.preventDefault()
                    if (passwordLocked || !recoveryPassword) return
                    void run('recoveryIssue', async () => {
                      const code = await regenerateRecovery(recoveryPassword)
                      setRecoveryPassword('')
                      if (code) setFreshCode(code)
                    })
                  }}
                >
                  <label className="block text-xs text-gray-500" htmlFor="recovery-password">
                    {t('recoveryNeedsPassword')}
                  </label>
                  <PasswordInput
                    id="recovery-password"
                    value={recoveryPassword}
                    onChange={setRecoveryPassword}
                    placeholder={t('currentPasswordPlaceholder')}
                    autoComplete="current-password"
                    disabled={passwordLocked}
                    className="max-w-xs"
                  />
                  <div className="flex flex-wrap gap-2">
                    <Button
                      type="submit"
                      variant="outline"
                      disabled={passwordLocked || !recoveryPassword}
                    >
                      {label(
                        'recoveryIssue',
                        status.hasRecovery ? t('recoveryCodeReissue') : t('recoveryCodeCreate'),
                      )}
                    </Button>
                    {status.hasRecovery && (
                      <Button
                        type="button"
                        variant="ghost"
                        className="text-red-500 hover:text-red-600"
                        disabled={passwordLocked}
                        onClick={() => {
                          setRemoveCodePassword('')
                          setRemovingCode(true)
                        }}
                      >
                        {t('recoveryCodeRemove')}
                      </Button>
                    )}
                  </div>
                  {message('recoveryIssue', 'mt-0')}
                  {!removingCode && message('recoveryRemove', 'mt-0')}
                </form>
              </div>

              {/* Quick unlock. Off unless the owner turns it on, and one switch
                  puts the archive back to password-only. */}
              {status.quickAvailable && (
                <div>
                  <div className="flex items-start justify-between gap-4">
                    <div className="min-w-0">
                      <h5 className="flex items-center gap-2 text-sm font-medium text-gray-900">
                        <ScanFace className="h-4 w-4 text-gray-500" />
                        {t('quickTitle')}
                      </h5>
                      <p className="mt-1 text-xs text-gray-500">{t('quickDescription')}</p>
                    </div>
                    <Switch
                      checked={status.quickEnabled}
                      aria-label={t('quickTitle')}
                      aria-describedby={status.quickEnabled ? undefined : 'quick-password-label'}
                      // Turning it off needs nothing; turning it on needs the
                      // password typed in the field right under it.
                      disabled={busy || (!status.quickEnabled && (waiting || !quickPassword))}
                      onCheckedChange={(wanted) =>
                        void run(
                          'quick',
                          async () => {
                            if (wanted) {
                              await quickEnable(quickPassword)
                              setQuickPassword('')
                            } else {
                              await quickDisable()
                            }
                          },
                          wanted ? t('noticeQuickOn') : t('noticeQuickOff'),
                        )
                      }
                    />
                  </div>

                  {/* Right under the switch, so its grey state explains itself;
                      Enter here turns quick unlock on. */}
                  {!status.quickEnabled && (
                    <form
                      className="mt-3"
                      onSubmit={(event) => {
                        event.preventDefault()
                        if (passwordLocked || !quickPassword) return
                        void run(
                          'quick',
                          async () => {
                            await quickEnable(quickPassword)
                            setQuickPassword('')
                          },
                          t('noticeQuickOn'),
                        )
                      }}
                    >
                      <label
                        id="quick-password-label"
                        htmlFor="quick-password"
                        className="mb-1.5 block text-xs text-gray-500"
                      >
                        {t('quickNeedsPassword')}
                      </label>
                      <PasswordInput
                        id="quick-password"
                        value={quickPassword}
                        onChange={setQuickPassword}
                        placeholder={t('currentPasswordPlaceholder')}
                        autoComplete="current-password"
                        disabled={passwordLocked}
                        className="max-w-xs"
                      />
                    </form>
                  )}
                  {message('quick')}

                  {/* Said plainly next to the switch, not buried in a document:
                      this door is weaker than the password. */}
                  <p className="mt-3 rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-xs text-[var(--af-text-2)]">
                    {t('quickWarning')}
                  </p>
                </div>
              )}

              {/* Idle timeout. Never applies while a recording is running. */}
              <div>
                <label
                  htmlFor="auto-lock"
                  className="mb-1.5 flex items-center gap-2 text-sm font-medium text-gray-900"
                >
                  <Timer className="h-4 w-4 text-gray-500" />
                  {t('autoLockLabel')}
                </label>
                <p className="mb-2 text-xs text-gray-500">{t('autoLockHint')}</p>
                <select
                  id="auto-lock"
                  value={status.autoLockMinutes ?? 0}
                  disabled={busy}
                  onChange={(event) => {
                    const minutes = Number(event.target.value)
                    void run('autoLock', () => setAutoLock(minutes === 0 ? null : minutes))
                  }}
                  className="rounded-md border border-gray-200 bg-white px-3 py-2 text-sm text-gray-900 focus:border-blue-400 focus:outline-none"
                >
                  {AUTO_LOCK_CHOICES.map((minutes) => (
                    <option key={minutes} value={minutes}>
                      {minutes === 0 ? t('autoLockNever') : t('autoLockMinutes', { minutes })}
                    </option>
                  ))}
                </select>
                {message('autoLock')}
              </div>
            </Group>

            <Group title={t('keyBackupTitle')}>
              <div>
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
                    void run(
                      'keyBackup',
                      () => invoke('security_export_key_backup', { path: target }),
                      t('noticeKeyBackupSaved'),
                    )
                  }}
                >
                  {label('keyBackup', t('keyBackupAction'))}
                </Button>
                {message('keyBackup')}
              </div>
            </Group>

            {/* Last, and behind a confirmation that asks for the password:
                nothing on the panel undoes more. */}
            <Group title={t('sectionDisable')}>
              <div>
                <p className="mb-3 text-xs text-gray-500">{t('disableProtectionHint')}</p>
                <Button
                  type="button"
                  variant="outline"
                  className="border-red-500/40 text-red-500 hover:text-red-600"
                  disabled={passwordLocked}
                  onClick={() => {
                    setDisablePassword('')
                    setDisabling(true)
                  }}
                >
                  <ShieldOff className="mr-2 h-4 w-4" />
                  {t('disableProtection')}
                </Button>
                {!disabling && message('disable')}
              </div>
            </Group>
          </div>
        )}
      </div>

      <div className="rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
        <div className="flex items-start gap-3">
          <Globe className="mt-0.5 h-5 w-5 shrink-0 text-blue-500" />
          <h3 className="text-lg font-semibold text-gray-900">{t('sectionNetwork')}</h3>
        </div>
        <div className="mt-5 space-y-5">
          <LocalOnlyCard />
          <CloudPrivacyCard available={configured} />
        </div>
      </div>

      <ConfirmDialog
        open={removingCode}
        onOpenChange={(open) => {
          if (!open) closeDialog('recoveryRemove', () => setRemovingCode(false))
        }}
        title={t('recoveryRemoveConfirmTitle')}
        description={t('recoveryRemoveConfirmBody')}
        confirmLabel={t('recoveryCodeRemove')}
        busyLabel={t('working')}
        busy={panel.running === 'recoveryRemove'}
        confirmDisabled={waiting || !removeCodePassword}
        destructive
        onConfirm={() =>
          void run(
            'recoveryRemove',
            async () => {
              await removeRecovery(removeCodePassword)
              setRemoveCodePassword('')
              setRemovingCode(false)
            },
            t('noticeRecoveryRemoved'),
            // The dialog is closed by then; the notice goes under the section.
            'recoveryIssue',
          )
        }
      >
        <PasswordInput
          value={removeCodePassword}
          onChange={setRemoveCodePassword}
          placeholder={t('currentPasswordPlaceholder')}
          autoComplete="current-password"
          disabled={passwordLocked}
        />
        {message('recoveryRemove', 'mt-0')}
      </ConfirmDialog>

      <ConfirmDialog
        open={disabling}
        onOpenChange={(open) => {
          if (!open) closeDialog('disable', () => setDisabling(false))
        }}
        title={t('disableProtectionConfirmTitle')}
        description={t('disableProtectionConfirmBody')}
        confirmLabel={t('disableProtection')}
        busyLabel={t('working')}
        busy={panel.running === 'disable'}
        confirmDisabled={waiting || !disablePassword}
        destructive
        onConfirm={() =>
          void run(
            'disable',
            async () => {
              await disable(disablePassword)
              setDisablePassword('')
              setDisabling(false)
            },
            t('noticeProtectionOff'),
            // The section is gone once protection is off; say so at the top.
            'setup',
          )
        }
      >
        <PasswordInput
          value={disablePassword}
          onChange={setDisablePassword}
          placeholder={t('currentPasswordPlaceholder')}
          autoComplete="current-password"
          disabled={passwordLocked}
        />
        {message('disable', 'mt-0')}
      </ConfirmDialog>
    </div>
  )
}

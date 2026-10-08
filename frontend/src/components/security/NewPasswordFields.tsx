'use client'

/**
 * A new password typed twice, with the rules shown while it is typed.
 *
 * Shared by every form that sets a password, so none of them can take a new
 * password from a single field: a typo there locks the archive behind a
 * password nobody knows. The form decides whether to submit from
 * `checkNewPassword`; this only draws the fields and says what is missing.
 */

import { forwardRef, useId, useState } from 'react'
import { useTranslations } from 'next-intl'
import { Eye, EyeOff } from 'lucide-react'
import { Input } from '@/components/ui/input'
import { MIN_PASSWORD_LENGTH, checkNewPassword } from '@/lib/password-rules'

/** A password field with its own show/hide toggle. */
const PasswordInput = forwardRef<
  HTMLInputElement,
  {
    value: string
    onChange: (value: string) => void
    placeholder: string
    disabled?: boolean
    invalid?: boolean
    describedBy?: string
  }
>(function PasswordInput({ value, onChange, placeholder, disabled, invalid, describedBy }, ref) {
  const t = useTranslations('security')
  const [reveal, setReveal] = useState(false)
  return (
    <div className="relative">
      <Input
        ref={ref}
        type={reveal ? 'text' : 'password'}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        autoComplete="new-password"
        disabled={disabled}
        aria-label={placeholder}
        aria-invalid={invalid || undefined}
        aria-describedby={describedBy}
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
  )
})

export function NewPasswordFields({
  password,
  repeat,
  onPasswordChange,
  onRepeatChange,
  disabled,
}: {
  password: string
  repeat: string
  onPasswordChange: (value: string) => void
  onRepeatChange: (value: string) => void
  disabled?: boolean
}) {
  const t = useTranslations('security')
  const check = checkNewPassword(password, repeat)
  const id = useId()
  const lengthHint = `${id}-length`
  const matchHint = `${id}-match`
  const showLength = password.length === 0 || check.tooShort

  return (
    <div className="space-y-3">
      <PasswordInput
        value={password}
        onChange={onPasswordChange}
        placeholder={t('newPasswordPlaceholder')}
        disabled={disabled}
        invalid={check.tooShort}
        describedBy={showLength ? lengthHint : undefined}
      />
      <PasswordInput
        value={repeat}
        onChange={onRepeatChange}
        placeholder={t('confirmPasswordPlaceholder')}
        disabled={disabled}
        invalid={check.mismatch}
        describedBy={check.mismatch ? matchHint : undefined}
      />
      {/* The minimum is said before anything is typed, not after a refusal;
          it turns to a warning only once a short password is actually there. */}
      <div className="space-y-1 text-xs" aria-live="polite">
        {showLength && (
          <p
            id={lengthHint}
            className={check.tooShort ? 'text-amber-600' : 'text-[var(--af-text-2)]'}
          >
            {t('passwordMinimumHint', { minimum: MIN_PASSWORD_LENGTH })}
          </p>
        )}
        {check.mismatch && (
          <p id={matchHint} className="text-amber-600">
            {t('errorPasswordsDoNotMatch')}
          </p>
        )}
      </div>
    </div>
  )
}

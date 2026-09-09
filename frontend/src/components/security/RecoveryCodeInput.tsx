'use client'

/**
 * A field for typing a recovery code off a sheet of paper.
 *
 * Groups characters as they are typed and drops anything the alphabet does not
 * contain, so the owner sees the same shape they are reading from. The backend
 * normalises again and has the last word — this is a courtesy, not a validator.
 */

import { useCallback } from 'react'
import { useTranslations } from 'next-intl'

/** Characters between dashes, matching how the code is shown. */
const GROUP = 4
/** Characters in a whole code. */
const LENGTH = 32
/** Crockford base32, plus the look-alikes the backend maps back. */
const ALLOWED = /[0-9A-HJKMNP-TV-Zilo]/i

export function RecoveryCodeInput({
  value,
  onChange,
  disabled,
}: {
  value: string
  onChange: (value: string) => void
  disabled?: boolean
}) {
  const t = useTranslations('security')

  const handle = useCallback(
    (raw: string) => {
      const kept = Array.from(raw)
        .filter((character) => ALLOWED.test(character))
        .slice(0, LENGTH)
        .join('')
        .toUpperCase()

      const grouped = kept.match(new RegExp(`.{1,${GROUP}}`, 'g'))?.join('-') ?? ''
      onChange(grouped)
    },
    [onChange],
  )

  return (
    <input
      type="text"
      value={value}
      onChange={(event) => handle(event.target.value)}
      placeholder="XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX"
      autoComplete="off"
      autoCapitalize="characters"
      spellCheck={false}
      disabled={disabled}
      aria-label={t('recoveryCodeLabel')}
      className="w-full rounded-md border border-[var(--af-border)] bg-[var(--af-panel)] px-3 py-2 font-mono text-sm tracking-tight text-[var(--af-text)] placeholder:text-[var(--af-text-2)] focus:border-[var(--af-accent)] focus:outline-none disabled:opacity-50"
    />
  )
}

'use client'

/**
 * A password field with its own show/hide toggle.
 *
 * The browser's built-in reveal button (`::-ms-reveal` in WebView2) is hidden:
 * it shows only while the field has text and focus, and two eyes side by side
 * read as a glitch.
 */

import { forwardRef, useState } from 'react'
import { useTranslations } from 'next-intl'
import { Eye, EyeOff } from 'lucide-react'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'

export const PasswordInput = forwardRef<
  HTMLInputElement,
  {
    value: string
    onChange: (value: string) => void
    placeholder: string
    /** `new-password` for a password being set, `current-password` for one being proved. */
    autoComplete: 'new-password' | 'current-password'
    disabled?: boolean
    invalid?: boolean
    describedBy?: string
    id?: string
    className?: string
  }
>(function PasswordInput(
  { value, onChange, placeholder, autoComplete, disabled, invalid, describedBy, id, className },
  ref,
) {
  const t = useTranslations('security')
  const [reveal, setReveal] = useState(false)
  return (
    <div className={cn('relative', className)}>
      <Input
        ref={ref}
        id={id}
        type={reveal ? 'text' : 'password'}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        autoComplete={autoComplete}
        disabled={disabled}
        aria-label={placeholder}
        aria-invalid={invalid || undefined}
        aria-describedby={describedBy}
        className="pr-10 [&::-ms-reveal]:hidden"
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

'use client'

/**
 * The recovery code, shown the one time it exists in readable form.
 *
 * It is never stored in the clear and cannot be asked for again — only replaced
 * — so this screen refuses to be dismissed until the owner confirms they have
 * put it somewhere. That is friction on purpose.
 */

import { useState } from 'react'
import { useTranslations } from 'next-intl'
import { Copy, Check, Printer, ShieldAlert } from 'lucide-react'
import { Button } from '@/components/ui/button'

export function RecoveryCodeCard({
  code,
  onConfirmed,
}: {
  code: string
  onConfirmed: () => void
}) {
  const t = useTranslations('security')
  const [copied, setCopied] = useState(false)
  const [acknowledged, setAcknowledged] = useState(false)

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(code)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 2000)
    } catch (error) {
      console.error('[Security] Could not copy the recovery code:', error)
    }
  }

  // Prints just the code and what it is for, in a window of its own, so the page
  // around it does not end up on the paper.
  const print = () => {
    const sheet = window.open('', '_blank', 'width=600,height=400')
    if (!sheet) return

    sheet.document.write(`
      <html>
        <head><title>${escapeHtml(t('recoveryPrintTitle'))}</title></head>
        <body style="font-family: system-ui, sans-serif; padding: 48px; color: #111;">
          <h1 style="font-size: 18px; margin: 0 0 8px;">${escapeHtml(t('recoveryPrintTitle'))}</h1>
          <p style="font-size: 13px; color: #444; margin: 0 0 24px; max-width: 46em;">
            ${escapeHtml(t('recoveryPrintExplanation'))}
          </p>
          <pre style="font-size: 20px; letter-spacing: 1px; font-family: ui-monospace, monospace; margin: 0 0 24px;">${escapeHtml(code)}</pre>
          <p style="font-size: 12px; color: #666;">${escapeHtml(t('recoveryPrintDate', { date: new Date().toLocaleDateString() }))}</p>
        </body>
      </html>
    `)
    sheet.document.close()
    sheet.focus()
    sheet.print()
  }

  return (
    <div className="space-y-5">
      <div className="flex gap-3 rounded-lg border border-amber-500/40 bg-amber-500/10 p-4">
        <ShieldAlert className="mt-0.5 h-5 w-5 shrink-0 text-amber-500" />
        <div className="text-sm text-[var(--af-text)]">
          <p className="font-medium">{t('recoveryShownOnceTitle')}</p>
          <p className="mt-1 text-[var(--af-text-2)]">{t('recoveryShownOnceBody')}</p>
        </div>
      </div>

      <div className="rounded-lg border border-[var(--af-border)] bg-[var(--af-panel)] p-4">
        <pre className="select-all whitespace-pre-wrap break-all text-center font-mono text-base leading-7 tracking-wide text-[var(--af-text)]">
          {code}
        </pre>
      </div>

      <div className="flex gap-2">
        <Button type="button" variant="outline" className="flex-1" onClick={copy}>
          {copied ? <Check className="mr-2 h-4 w-4" /> : <Copy className="mr-2 h-4 w-4" />}
          {copied ? t('copied') : t('copy')}
        </Button>
        <Button type="button" variant="outline" className="flex-1" onClick={print}>
          <Printer className="mr-2 h-4 w-4" />
          {t('print')}
        </Button>
      </div>

      <label className="flex cursor-pointer items-start gap-2.5 text-sm text-[var(--af-text-2)]">
        <input
          type="checkbox"
          checked={acknowledged}
          onChange={(event) => setAcknowledged(event.target.checked)}
          className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--af-accent)]"
        />
        <span>{t('recoveryAcknowledge')}</span>
      </label>

      <Button className="w-full" disabled={!acknowledged} onClick={onConfirmed}>
        {t('recoveryDone')}
      </Button>
    </div>
  )
}

/** The code is generated locally, but the label texts come from translations. */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

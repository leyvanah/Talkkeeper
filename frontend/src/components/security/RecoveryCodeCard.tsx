'use client'

/**
 * The recovery code, shown the one time it exists in readable form.
 *
 * It is never stored in the clear and cannot be asked for again — only replaced
 * — so this screen refuses to be dismissed until the owner confirms they have
 * put it somewhere. That is friction on purpose.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslations } from 'next-intl'
import { Copy, Check, Printer, ShieldAlert } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { toastFailure } from '@/lib/failure'
import { copyText } from '@/lib/private-clipboard'
import { recoverySheetHtml, watchPrint } from '@/lib/recovery-print'

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
      await copyText(code)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 2000)
    } catch (error) {
      console.error('[Security] Could not copy the recovery code:', error)
    }
  }

  // The hidden frame the sheet is printed from, and what stops waiting for its
  // dialog. Both go as soon as printing is over, so the code does not stay in
  // the page any longer than the screen shows it.
  const sheetFrame = useRef<HTMLIFrameElement | null>(null)
  const stopWatching = useRef<(() => void) | null>(null)

  const removeSheet = useCallback(() => {
    stopWatching.current?.()
    stopWatching.current = null
    sheetFrame.current?.remove()
    sheetFrame.current = null
  }, [])

  useEffect(() => removeSheet, [removeSheet])

  // Prints just the code and what it is for from a frame of its own, so the
  // page around it does not end up on the paper.
  const print = () => {
    removeSheet()
    const failed = (error: unknown) => {
      removeSheet()
      toastFailure(t('printFailed'), 'recovery-print', error)
    }

    try {
      const frame = document.createElement('iframe')
      frame.setAttribute('aria-hidden', 'true')
      frame.tabIndex = -1
      // Off screen rather than `display: none`, which may leave it nothing to print.
      frame.style.cssText = 'position: fixed; left: -10000px; top: 0; width: 600px; height: 400px; border: 0;'
      document.body.appendChild(frame)
      sheetFrame.current = frame

      const sheet = frame.contentWindow
      if (!sheet) throw new Error('The print frame has no window')
      sheet.document.open()
      sheet.document.write(
        recoverySheetHtml({
          title: t('recoveryPrintTitle'),
          explanation: t('recoveryPrintExplanation'),
          code,
          issued: t('recoveryPrintDate', { date: new Date().toLocaleDateString() }),
        }),
      )
      sheet.document.close()

      const stop = watchPrint(sheet, { onDone: removeSheet, onFailed: failed }, (ms, run) => {
        const timer = window.setTimeout(run, ms)
        return () => window.clearTimeout(timer)
      })
      // A dialog that has already closed took the frame with it.
      if (sheetFrame.current === frame) stopWatching.current = stop
    } catch (error) {
      failed(error)
    }
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

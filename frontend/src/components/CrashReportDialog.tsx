'use client'

import { useState } from 'react'
import { useLocale, useTranslations } from 'next-intl'
import { FileArchive, Loader2, Send, ShieldCheck } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  chooseCrashReportDestination,
  createCrashReportZip,
  dismissCrashReport,
  openCrashReportIssue,
  type PendingCrashReport,
} from '@/services/crashReportService'

interface CrashReportDialogProps {
  report: PendingCrashReport
  onResolved: () => void
}

type PendingAction = 'send' | 'save' | 'ignore' | null

export default function CrashReportDialog({ report, onResolved }: CrashReportDialogProps) {
  const t = useTranslations('app')
  const locale = useLocale()
  const [pendingAction, setPendingAction] = useState<PendingAction>(null)
  const busy = pendingAction !== null

  const createZip = async () => {
    const destination = await chooseCrashReportDestination(report)
    if (!destination) return null
    return createCrashReportZip(destination)
  }

  const finishDialog = async () => {
    try {
      await dismissCrashReport()
    } catch (error) {
      console.error('[CrashReport] Failed to persist dismissal:', error)
      toast.warning(t('crashDismissFailed'), {
        description: 'You can continue now, but Talkkeeper may ask about it again next launch.',
      })
    }
    onResolved()
  }

  const handleSave = async () => {
    setPendingAction('save')
    try {
      const destination = await createZip()
      if (!destination) return
      await finishDialog()
      toast.success(t('crashSaved'), { description: destination })
    } catch (error) {
      console.error('[CrashReport] Failed to save report:', error)
      toast.error(t('crashSaveFailed'))
    } finally {
      setPendingAction(null)
    }
  }

  const handleSend = async () => {
    setPendingAction('send')
    try {
      const destination = await createZip()
      if (!destination) return
      await openCrashReportIssue(report)
      await finishDialog()
      toast.success(t('crashReady'), {
        description: 'GitHub opened with the report details. Attach the ZIP you just saved.',
      })
    } catch (error) {
      console.error('[CrashReport] Failed to prepare report:', error)
      toast.error(t('crashPrepareFailed'))
    } finally {
      setPendingAction(null)
    }
  }

  const handleIgnore = async () => {
    setPendingAction('ignore')
    try {
      await finishDialog()
    } finally {
      setPendingAction(null)
    }
  }

  const detected = new Date(report.detectedAt)
  const detectedLabel = Number.isNaN(detected.getTime())
    ? t('crashPreviousSession')
    : detected.toLocaleString(locale)
  const description = report.crashType === 'panic'
    ? t('crashDescriptionPanic')
    : t('crashDescriptionUnclean')

  return (
    <Dialog open>
      <DialogContent
        showCloseButton={false}
        onEscapeKeyDown={(event) => event.preventDefault()}
        onPointerDownOutside={(event) => event.preventDefault()}
        className="max-h-[calc(100vh-2rem)] max-w-[520px] gap-0 overflow-y-auto border-[var(--af-border)] bg-[var(--af-panel)] p-0 shadow-2xl"
      >
        <div className="border-b border-[var(--af-border)] bg-gradient-to-br from-red-500/10 via-transparent to-transparent px-6 py-5">
          <DialogHeader className="text-left">
            <div className="mb-3 flex h-10 w-10 items-center justify-center rounded-xl border border-red-400/25 bg-red-500/10 text-red-300">
              <ShieldCheck className="h-5 w-5" />
            </div>
            <DialogTitle className="text-xl text-[var(--af-text)]">
              {t('crashTitle')}
            </DialogTitle>
            <DialogDescription className="text-[var(--af-text-2)]">
              {description} {t('crashDescriptionSuffix')}
            </DialogDescription>
          </DialogHeader>
        </div>

        <div className="space-y-4 px-6 py-5">
          <div className="rounded-lg border border-[var(--af-border)] bg-black/10 px-4 py-3 text-sm">
            <div className="flex items-center justify-between gap-4">
              <span className="text-[var(--af-text-3)]">{t('crashDetected')}</span>
              <span className="text-right text-[var(--af-text-2)]">{detectedLabel}</span>
            </div>
            <div className="mt-2 flex items-center justify-between gap-4">
              <span className="text-[var(--af-text-3)]">{t('crashVersion')}</span>
              <span className="font-mono text-[var(--af-text-2)]">{report.appVersion}</span>
            </div>
          </div>

          <details className="group rounded-lg border border-[var(--af-border)] bg-black/10 px-4 py-3 text-sm">
            <summary className="cursor-pointer select-none rounded font-medium text-[var(--af-text-2)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)]">
              {t('crashWhatsIncluded')}
            </summary>
            <div className="mt-3 space-y-2 border-t border-[var(--af-border)] pt-3 text-xs leading-relaxed text-[var(--af-text-3)]">
              <p>{t('crashIncludes')}</p>
              <p>{t('crashExcludes')}</p>
            </div>
          </details>

          <p className="text-xs leading-relaxed text-[var(--af-text-3)]">
            {t('crashSendNote')}
          </p>

          <div className="grid grid-cols-1 gap-2 pt-1 sm:grid-cols-3">
            <Button variant="ghost" onClick={handleIgnore} disabled={busy}>
              {pendingAction === 'ignore' && <Loader2 className="animate-spin" />}
              {t('crashIgnore')}
            </Button>
            <Button variant="outline" onClick={handleSave} disabled={busy}>
              {pendingAction === 'save' ? <Loader2 className="animate-spin" /> : <FileArchive />}
              {t('crashSaveZip')}
            </Button>
            <Button onClick={handleSend} disabled={busy}>
              {pendingAction === 'send' ? <Loader2 className="animate-spin" /> : <Send />}
              {t('crashSend')}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

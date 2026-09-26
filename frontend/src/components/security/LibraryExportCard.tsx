'use client'

/**
 * Taking the whole library out of the application.
 *
 * The readable export writes every recording as ordinary files — audio as it
 * was recorded, transcript and summary as text — decrypted. That is what it is
 * for, and exactly why it asks first: anyone who can open the folder can read
 * and hear everything in it.
 */

import { useEffect, useState } from 'react'
import { useTranslations } from 'next-intl'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { AlertTriangle, FolderOutput } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Progress } from '@/components/ui/progress'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'

interface ExportReport {
  folder: string
  meetings: number
  audioFiles: number
  audioBytes: number
  failed: number
}

export function LibraryExportCard() {
  const t = useTranslations('security')
  const [confirming, setConfirming] = useState(false)
  const [running, setRunning] = useState(false)
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null)
  const [report, setReport] = useState<ExportReport | null>(null)

  useEffect(() => {
    const unlisten = listen<{ done: number; total: number }>('library-export-progress', (event) => {
      setProgress(event.payload)
    })
    return () => {
      void unlisten.then((stop) => stop())
    }
  }, [])

  const exportReadable = async () => {
    setConfirming(false)
    // The folder is asked for before anything starts: closing the picker
    // leaves nothing half-written.
    const target = await openDialog({ directory: true, multiple: false })
    if (typeof target !== 'string') return
    setRunning(true)
    setReport(null)
    setProgress(null)
    try {
      const result = await invoke<ExportReport>('export_library_readable', {
        targetDir: target,
        unassignedLabel: t('libraryExportUnassigned'),
      })
      setReport(result)
      if (result.failed > 0) {
        toast.warning(t('libraryExportPartial', { failed: result.failed }))
      } else {
        toast.success(t('libraryExportDone'))
      }
    } catch (error) {
      console.error('[LibraryExport] Export failed:', error)
      toast.error(t('libraryExportFailed'), {
        description: typeof error === 'string' ? error : String(error),
      })
    } finally {
      setRunning(false)
    }
  }

  const percent = progress && progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0

  return (
    <div className="rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
      <div className="flex items-start gap-3">
        <FolderOutput className="mt-0.5 h-5 w-5 shrink-0 text-blue-500" />
        <div className="min-w-0 flex-1">
          <h3 className="text-lg font-semibold text-gray-900">{t('libraryExportTitle')}</h3>
          <p className="mt-1 text-sm text-gray-600">{t('libraryExportDescription')}</p>
        </div>
      </div>

      <div className="mt-4 space-y-3">
        <Button type="button" variant="outline" disabled={running} onClick={() => setConfirming(true)}>
          {t('libraryExportReadable')}
        </Button>

        {running && (
          <div className="space-y-1">
            <Progress value={percent} />
            <p className="text-xs text-gray-500">
              {progress
                ? t('libraryExportProgress', { done: progress.done, total: progress.total })
                : t('libraryExportPreparing')}
            </p>
          </div>
        )}

        {report && !running && (
          <div className="rounded-md bg-gray-50 p-3 text-xs text-gray-600">
            <p>
              {t('libraryExportSummary', {
                meetings: report.meetings,
                audioFiles: report.audioFiles,
                megabytes: Math.round(report.audioBytes / 1_048_576),
              })}
            </p>
            <p className="mt-1 break-all font-mono">{report.folder}</p>
          </div>
        )}
      </div>

      <Dialog open={confirming} onOpenChange={setConfirming}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <AlertTriangle className="h-5 w-5 text-amber-500" />
              {t('libraryExportConfirmTitle')}
            </DialogTitle>
            <DialogDescription>{t('libraryExportConfirmText')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => setConfirming(false)}>
              {t('libraryExportCancel')}
            </Button>
            <Button type="button" onClick={() => void exportReadable()}>
              {t('libraryExportConfirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}

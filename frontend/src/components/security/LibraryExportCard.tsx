'use client'

/**
 * Taking the whole library out of the application, and bringing it into
 * another installation.
 *
 * The readable export writes every recording as ordinary files — audio as it
 * was recorded, transcript and summary as text — decrypted. That is what it is
 * for, and exactly why it asks first: anyone who can open the folder can read
 * and hear everything in it.
 *
 * The transfer package is the same library sealed with a password chosen for
 * the transfer. Another installation imports it with that password and seals
 * everything again with its own key.
 */

import { useEffect, useState } from 'react'
import { useTranslations } from 'next-intl'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { AlertTriangle, FolderInput, FolderOutput, Package } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Progress } from '@/components/ui/progress'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'

const MIN_PASSWORD = 8

interface ExportReport {
  folder: string
  meetings: number
  audioFiles: number
  audioBytes: number
  failed: number
}

interface ImportReport {
  imported: number
  alreadyPresent: number
  failed: number
}

type Step = 'readable' | 'packagePassword' | 'importPassword' | null
type Running = 'export' | 'import' | null

const describe = (error: unknown) => (typeof error === 'string' ? error : String(error))

async function pickFolder(): Promise<string | null> {
  const chosen = await openDialog({ directory: true, multiple: false })
  return typeof chosen === 'string' ? chosen : null
}

export function LibraryExportCard() {
  const t = useTranslations('security')
  const [step, setStep] = useState<Step>(null)
  const [running, setRunning] = useState<Running>(null)
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null)
  const [exportReport, setExportReport] = useState<ExportReport | null>(null)
  const [importReport, setImportReport] = useState<ImportReport | null>(null)
  const [password, setPassword] = useState('')
  const [repeat, setRepeat] = useState('')
  const [packageDir, setPackageDir] = useState<string | null>(null)

  useEffect(() => {
    const stops = ['library-export-progress', 'library-import-progress'].map((event) =>
      listen<{ done: number; total: number }>(event, (payload) => setProgress(payload.payload)),
    )
    return () => {
      stops.forEach((stop) => void stop.then((unlisten) => unlisten()))
    }
  }, [])

  const closeDialog = () => {
    setStep(null)
    setPassword('')
    setRepeat('')
  }

  const begin = (kind: Running) => {
    setRunning(kind)
    setProgress(null)
    setExportReport(null)
    setImportReport(null)
  }

  const finishExport = (result: ExportReport) => {
    setExportReport(result)
    if (result.failed > 0) toast.warning(t('libraryExportPartial', { failed: result.failed }))
    else toast.success(t('libraryExportDone'))
  }

  const exportReadable = async () => {
    closeDialog()
    // The folder is asked for before anything starts: closing the picker
    // leaves nothing half-written.
    const target = await pickFolder()
    if (!target) return
    begin('export')
    try {
      finishExport(
        await invoke<ExportReport>('export_library_readable', {
          targetDir: target,
          unassignedLabel: t('libraryExportUnassigned'),
        }),
      )
    } catch (error) {
      console.error('[LibraryExport] Export failed:', error)
      toast.error(t('libraryExportFailed'), { description: describe(error) })
    } finally {
      setRunning(null)
    }
  }

  const exportPackage = async () => {
    const chosen = password
    closeDialog()
    const target = await pickFolder()
    if (!target) return
    begin('export')
    try {
      finishExport(await invoke<ExportReport>('export_library_package', { targetDir: target, password: chosen }))
    } catch (error) {
      console.error('[LibraryExport] Package export failed:', error)
      toast.error(t('libraryExportFailed'), { description: describe(error) })
    } finally {
      setRunning(null)
    }
  }

  const chooseImport = async () => {
    const chosen = await pickFolder()
    if (!chosen) return
    setPackageDir(chosen)
    setStep('importPassword')
  }

  const importPackage = async () => {
    const typed = password
    closeDialog()
    if (!packageDir) return
    begin('import')
    try {
      const result = await invoke<ImportReport>('import_library_package', { packageDir, password: typed })
      setImportReport(result)
      if (result.failed > 0) toast.warning(t('libraryImportPartial', { failed: result.failed }))
      else toast.success(t('libraryImportDone'))
    } catch (error) {
      console.error('[LibraryExport] Import failed:', error)
      toast.error(t('libraryImportFailed'), { description: describe(error) })
    } finally {
      setRunning(null)
    }
  }

  const percent = progress && progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0
  const passwordTooShort = password.length > 0 && password.length < MIN_PASSWORD
  const passwordsDiffer = repeat.length > 0 && repeat !== password
  const packageReady = password.length >= MIN_PASSWORD && repeat === password

  return (
    <div className="rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
      <div className="flex items-start gap-3">
        <FolderOutput className="mt-0.5 h-5 w-5 shrink-0 text-blue-500" />
        <div className="min-w-0 flex-1">
          <h3 className="text-lg font-semibold text-gray-900">{t('libraryExportTitle')}</h3>
          <p className="mt-1 text-sm text-gray-600">{t('libraryExportDescription')}</p>
        </div>
      </div>

      <div className="mt-4 space-y-4">
        <div>
          <Button type="button" variant="outline" disabled={running !== null} onClick={() => setStep('readable')}>
            <FolderOutput className="mr-2 h-4 w-4" />
            {t('libraryExportReadable')}
          </Button>
          <p className="mt-1 text-xs text-gray-500">{t('libraryExportReadableHint')}</p>
        </div>

        <div>
          <Button type="button" variant="outline" disabled={running !== null} onClick={() => setStep('packagePassword')}>
            <Package className="mr-2 h-4 w-4" />
            {t('libraryExportPackage')}
          </Button>
          <p className="mt-1 text-xs text-gray-500">{t('libraryExportPackageHint')}</p>
        </div>

        <div>
          <Button type="button" variant="outline" disabled={running !== null} onClick={() => void chooseImport()}>
            <FolderInput className="mr-2 h-4 w-4" />
            {t('libraryImportPackage')}
          </Button>
          <p className="mt-1 text-xs text-gray-500">{t('libraryImportPackageHint')}</p>
        </div>

        {running && (
          <div className="space-y-1">
            <Progress value={percent} />
            <p className="text-xs text-gray-500">
              {progress
                ? t(running === 'import' ? 'libraryImportProgress' : 'libraryExportProgress', {
                    done: progress.done,
                    total: progress.total,
                  })
                : t('libraryExportPreparing')}
            </p>
          </div>
        )}

        {exportReport && !running && (
          <div className="rounded-md bg-gray-50 p-3 text-xs text-gray-600">
            <p>
              {t('libraryExportSummary', {
                meetings: exportReport.meetings,
                audioFiles: exportReport.audioFiles,
                megabytes: Math.round(exportReport.audioBytes / 1_048_576),
              })}
            </p>
            <p className="mt-1 break-all font-mono">{exportReport.folder}</p>
          </div>
        )}

        {importReport && !running && (
          <div className="rounded-md bg-gray-50 p-3 text-xs text-gray-600">
            {t('libraryImportSummary', {
              imported: importReport.imported,
              alreadyPresent: importReport.alreadyPresent,
            })}
          </div>
        )}
      </div>

      <Dialog open={step === 'readable'} onOpenChange={(open) => !open && closeDialog()}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <AlertTriangle className="h-5 w-5 text-amber-500" />
              {t('libraryExportConfirmTitle')}
            </DialogTitle>
            <DialogDescription>{t('libraryExportConfirmText')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={closeDialog}>
              {t('libraryExportCancel')}
            </Button>
            <Button type="button" onClick={() => void exportReadable()}>
              {t('libraryExportConfirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={step === 'packagePassword'} onOpenChange={(open) => !open && closeDialog()}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('libraryPackagePasswordTitle')}</DialogTitle>
            <DialogDescription>{t('libraryPackagePasswordText', { min: MIN_PASSWORD })}</DialogDescription>
          </DialogHeader>
          <form
            className="space-y-3"
            onSubmit={(event) => {
              event.preventDefault()
              if (packageReady) void exportPackage()
            }}
          >
            <Input
              type="password"
              autoComplete="new-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder={t('libraryPackagePassword')}
              aria-label={t('libraryPackagePassword')}
            />
            <Input
              type="password"
              autoComplete="new-password"
              value={repeat}
              onChange={(event) => setRepeat(event.target.value)}
              placeholder={t('libraryPackagePasswordRepeat')}
              aria-label={t('libraryPackagePasswordRepeat')}
            />
            {(passwordTooShort || passwordsDiffer) && (
              <p className="text-xs text-red-500">
                {passwordTooShort
                  ? t('libraryPackagePasswordShort', { min: MIN_PASSWORD })
                  : t('libraryPackagePasswordMismatch')}
              </p>
            )}
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={closeDialog}>
                {t('libraryExportCancel')}
              </Button>
              <Button type="submit" disabled={!packageReady}>
                {t('libraryPackageChooseFolder')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={step === 'importPassword'} onOpenChange={(open) => !open && closeDialog()}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('libraryImportPasswordTitle')}</DialogTitle>
            <DialogDescription>{t('libraryImportPasswordText')}</DialogDescription>
          </DialogHeader>
          <form
            className="space-y-3"
            onSubmit={(event) => {
              event.preventDefault()
              if (password) void importPackage()
            }}
          >
            <Input
              type="password"
              autoComplete="off"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder={t('libraryPackagePassword')}
              aria-label={t('libraryPackagePassword')}
            />
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={closeDialog}>
                {t('libraryExportCancel')}
              </Button>
              <Button type="submit" disabled={!password}>
                {t('libraryImportStart')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  )
}

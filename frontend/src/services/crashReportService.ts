import { invoke } from '@tauri-apps/api/core'
import { save } from '@tauri-apps/plugin-dialog'

export type CrashType = 'panic' | 'unexpected_exit'

export interface PendingCrashReport {
  reportId: string
  detectedAt: string
  crashType: CrashType
  appVersion: string
}

export async function getPendingCrashReport(): Promise<PendingCrashReport | null> {
  return invoke<PendingCrashReport | null>('get_pending_crash_report')
}

export async function chooseCrashReportDestination(
  report: PendingCrashReport,
): Promise<string | null> {
  const destination = await save({
    defaultPath: `Talkkeeper-crash-${report.reportId.slice(0, 8)}.zip`,
    filters: [{ name: 'ZIP archive', extensions: ['zip'] }],
  })
  if (!destination) return null
  return destination.toLowerCase().endsWith('.zip') ? destination : `${destination}.zip`
}

export async function createCrashReportZip(destination: string): Promise<string> {
  return invoke<string>('create_crash_report_zip', { destination })
}

export async function dismissCrashReport(): Promise<void> {
  await invoke('dismiss_pending_crash_report')
}

export async function openCrashReportIssue(report: PendingCrashReport): Promise<void> {
  const title = 'Talkkeeper crash report'
  const body = [
    '## Crash report',
    '',
    'Please attach the ZIP Talkkeeper just created, then describe what was happening before the crash.',
  ].join('\n')
  const query = new URLSearchParams({
    title,
    body,
  })

  await invoke('open_external_url', {
    url: `https://github.com/leyvanah/Talkkeeper/issues/new?${query.toString()}`,
  })
}

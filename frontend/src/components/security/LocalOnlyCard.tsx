'use client'

/**
 * Local-only mode: whether summaries, the assistant and speech recognition may
 * talk to anything but this computer. The backend enforces it on every request
 * (`network_policy`); this section only shows and changes the setting.
 */

import { useEffect, useState } from 'react'
import { useTranslations } from 'next-intl'
import { invoke } from '@tauri-apps/api/core'
import { WifiOff } from 'lucide-react'
import { Switch } from '@/components/ui/switch'

export function LocalOnlyCard() {
  const t = useTranslations('security')
  const [enabled, setEnabled] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    invoke<boolean>('get_local_only_mode')
      .then(setEnabled)
      .catch((failure) => console.error('[LocalOnlyCard] Could not read the setting:', failure))
  }, [])

  const change = async (wanted: boolean) => {
    setBusy(true)
    setError(null)
    try {
      await invoke('set_local_only_mode', { enabled: wanted })
      setEnabled(wanted)
    } catch (failure) {
      console.error('[LocalOnlyCard] Could not save the setting:', failure)
      setError(t('localOnlySaveFailed'))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div>
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h4 className="flex items-center gap-2 text-sm font-medium text-gray-900">
            <WifiOff className="h-4 w-4 text-gray-500" />
            {t('localOnlyTitle')}
          </h4>
          <p className="mt-1 text-xs text-gray-500">{t('localOnlyDescription')}</p>
        </div>
        <Switch
          checked={enabled ?? true}
          disabled={busy || enabled === null}
          onCheckedChange={(wanted) => change(wanted)}
          aria-label={t('localOnlyTitle')}
        />
      </div>
      {enabled === false && <p className="mt-2 text-sm text-amber-500">{t('localOnlyOffWarning')}</p>}
      {error && (
        <p role="alert" className="mt-2 text-sm text-red-500">
          {error}
        </p>
      )}
    </div>
  )
}

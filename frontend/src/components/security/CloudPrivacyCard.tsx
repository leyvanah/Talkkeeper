'use client'

/**
 * Hiding names before text goes to a model that is not on this machine.
 *
 * Only that path: a local model reads the conversation as it is, and so does
 * anything written here for the owner. The card says which names the archive
 * already knows, so that the list is not a guess, and takes the words the
 * archive cannot know — places, nicknames, employers.
 */

import { useCallback, useEffect, useState } from 'react'
import { useTranslations } from 'next-intl'
import { invoke } from '@tauri-apps/api/core'
import { toast } from 'sonner'
import { Switch } from '@/components/ui/switch'
import { Button } from '@/components/ui/button'

interface PrivacyOverview {
  anonymizeCloud: boolean
  hiddenTerms: string[]
  knownNames: string[]
}

/** The owner's own name lives in the window, not in the archive. */
function ownName(): string {
  try {
    return localStorage.getItem('meetily_user_name')?.trim() ?? ''
  } catch {
    return ''
  }
}

export function CloudPrivacyCard() {
  const t = useTranslations('security')
  const [overview, setOverview] = useState<PrivacyOverview | null>(null)
  const [terms, setTerms] = useState('')
  const [saving, setSaving] = useState(false)

  const load = useCallback(async () => {
    try {
      const loaded = await invoke<PrivacyOverview>('api_get_privacy_settings')
      setOverview(loaded)
      // The owner's own name is worth hiding too, and only the window knows it.
      const mine = ownName()
      const lines = [...loaded.hiddenTerms]
      if (mine && !lines.some((term) => term.toLowerCase() === mine.toLowerCase())) {
        lines.push(mine)
      }
      setTerms(lines.join('\n'))
    } catch (error) {
      console.error('[Privacy] Failed to read settings:', error)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  const persist = async (next: Partial<PrivacyOverview>) => {
    if (!overview) return
    const anonymizeCloud = next.anonymizeCloud ?? overview.anonymizeCloud
    const hiddenTerms = (next.hiddenTerms ?? terms.split('\n'))
      .map((term) => term.trim())
      .filter(Boolean)
    setSaving(true)
    try {
      await invoke('api_save_privacy_settings', { anonymizeCloud, hiddenTerms })
      setOverview({ ...overview, anonymizeCloud, hiddenTerms })
      toast.success(t('cloudPrivacySaved'))
    } catch (error) {
      console.error('[Privacy] Failed to save settings:', error)
      toast.error(t('cloudPrivacySaveFailed'))
    } finally {
      setSaving(false)
    }
  }

  if (!overview) return null

  return (
    <div className="border-t border-gray-200 pt-5">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h4 className="mb-1 text-sm font-medium text-gray-900">{t('cloudPrivacyTitle')}</h4>
          <p className="text-xs text-gray-500">{t('cloudPrivacyDescription')}</p>
        </div>
        <Switch
          checked={overview.anonymizeCloud}
          disabled={saving}
          onCheckedChange={(checked) => void persist({ anonymizeCloud: checked })}
          aria-label={t('cloudPrivacyTitle')}
        />
      </div>

      {overview.anonymizeCloud && (
        <div className="mt-4 space-y-3">
          <div>
            <p className="text-xs font-medium text-gray-700">{t('cloudPrivacyKnown')}</p>
            <p className="mt-1 text-xs text-gray-500">
              {overview.knownNames.length > 0
                ? overview.knownNames.join(', ')
                : t('cloudPrivacyKnownEmpty')}
            </p>
          </div>

          <div>
            <label className="text-xs font-medium text-gray-700" htmlFor="privacy-terms">
              {t('cloudPrivacyTerms')}
            </label>
            <p className="mb-1 mt-1 text-xs text-gray-500">{t('cloudPrivacyTermsHint')}</p>
            <textarea
              id="privacy-terms"
              value={terms}
              onChange={(event) => setTerms(event.target.value)}
              rows={4}
              spellCheck={false}
              className="w-full rounded-md border border-[var(--af-border)] bg-[var(--af-panel)] p-2 text-sm text-[var(--af-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)]"
            />
            <div className="mt-2 flex flex-wrap gap-2">
              <Button type="button" variant="outline" disabled={saving} onClick={() => void persist({})}>
                {t('cloudPrivacyTermsSave')}
              </Button>
            </div>
          </div>

          <p className="text-xs text-gray-500">{t('cloudPrivacyLimits')}</p>
        </div>
      )}
    </div>
  )
}

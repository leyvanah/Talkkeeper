"use client"

import { useEffect, useState } from "react"
import { useTranslations } from "next-intl"
import { invoke } from "@tauri-apps/api/core"
import { Github, Shield, Cpu, Heart } from "lucide-react"

/**
 * About panel for Talkkeeper. Shows version, a short description,
 * and links to the source and privacy policy. Purely informational.
 */
export function AboutSettings() {
  const t = useTranslations('about');
  const [version, setVersion] = useState('0.0.1');

  useEffect(() => {
    (async () => {
      try {
        const { getVersion } = await import('@tauri-apps/api/app');
        setVersion(await getVersion());
      } catch {
        // Not in a Tauri context; keep the default.
      }
    })();
  }, []);

  const openUrl = (url: string) => {
    invoke('open_external_url', { url }).catch((e) => console.error('Failed to open URL:', e));
  };

  const REPO_URL = 'https://github.com/TylerBuza/Meetily-ActuallyFree';
  const ORIGINAL_MEETILY_URL = 'https://github.com/Zackriya-Solutions/meeting-minutes';
  const AUTHOR_URL = 'https://buza.dev';

  return (
    <div className="space-y-6">
      {/* Identity card */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <div className="flex items-center gap-4">
          <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-gradient-to-br from-blue-500 to-purple-600 text-white text-2xl font-bold shadow-sm">
            M
          </div>
          <div>
            <h3 className="text-xl font-semibold text-gray-900">Talkkeeper</h3>
            <p className="text-sm text-gray-600">
              {t('versionTagline', { version })}
            </p>
          </div>
        </div>
        <p className="mt-4 text-sm text-gray-600 leading-relaxed">
          {t('productSummary')}
        </p>
      </div>

      {/* Highlights */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
        <div className="bg-white rounded-lg border border-gray-200 p-4 shadow-sm">
          <Cpu className="w-5 h-5 text-blue-500 mb-2" />
          <div className="text-sm font-medium text-gray-900">{t('highlightOnDevice')}</div>
          <div className="text-xs text-gray-500">{t('highlightOnDeviceDetail')}</div>
        </div>
        <div className="bg-white rounded-lg border border-gray-200 p-4 shadow-sm">
          <Shield className="w-5 h-5 text-green-500 mb-2" />
          <div className="text-sm font-medium text-gray-900">{t('highlightPrivate')}</div>
          <div className="text-xs text-gray-500">{t('highlightPrivateDetail')}</div>
        </div>
        <div className="bg-white rounded-lg border border-gray-200 p-4 shadow-sm">
          <Heart className="w-5 h-5 text-pink-500 mb-2" />
          <div className="text-sm font-medium text-gray-900">{t('highlightFree')}</div>
          <div className="text-xs text-gray-500">{t('highlightFreeDetail')}</div>
        </div>
      </div>

      {/* Links */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm space-y-3">
        <h4 className="text-sm font-semibold text-gray-900">{t('links')}</h4>
        <button
          onClick={() => openUrl(REPO_URL)}
          className="flex items-center gap-3 w-full text-left px-3 py-2 rounded-md border border-gray-200 hover:border-blue-400 hover:bg-blue-50 transition-colors"
        >
          <Github className="w-4 h-4 text-gray-700" />
          <span className="text-sm text-gray-800">{t('sourceOnGithub')}</span>
        </button>
        <button
          onClick={() => openUrl(`${REPO_URL}/blob/main/PRIVACY_POLICY.md`)}
          className="flex items-center gap-3 w-full text-left px-3 py-2 rounded-md border border-gray-200 hover:border-blue-400 hover:bg-blue-50 transition-colors"
        >
          <Shield className="w-4 h-4 text-gray-700" />
          <span className="text-sm text-gray-800">{t('privacyPolicy')}</span>
        </button>
      </div>

      <div className="text-center text-xs text-gray-400 space-y-1">
        <p>
          {t('builtOnPrefix')}
          <button
            type="button"
            onClick={() => openUrl(ORIGINAL_MEETILY_URL)}
            className="text-blue-500 hover:underline"
          >
            {t('openSourceProject')}
          </button>
          {t('builtOnSuffix')}
        </p>
        <p>
          <button
            type="button"
            onClick={() => openUrl(AUTHOR_URL)}
            className="text-blue-500 hover:underline"
          >
            {t('forkCredit')}
          </button>
        </p>
      </div>
    </div>
  );
}

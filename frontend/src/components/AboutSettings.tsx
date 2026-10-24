"use client"

import { useEffect, useState } from "react"
import { useTranslations } from "next-intl"
import { invoke } from "@tauri-apps/api/core"
import { Github, Shield, Cpu, Heart, FileText, Download } from "lucide-react"
import Image from "next/image"

/**
 * About panel for Talkkeeper. Shows version, a short description,
 * and links to the source and privacy policy. Purely informational.
 * The same panel is shown in Settings and in the dialog behind the sidebar logo.
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

  const REPO_URL = 'https://github.com/leyvanah/Talkkeeper';
  const PROFILE_URL = 'https://github.com/leyvanah';

  return (
    <div className="space-y-6">
      {/* Identity card */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <div className="flex items-center gap-4">
          {/* Same artwork as the app icon (src-tauri/icons). */}
          <Image src="/icon_128x128.png" alt="" width={56} height={56} className="h-14 w-14" />
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
        {/* No update source of our own yet: point at the releases instead. */}
        <button
          onClick={() => openUrl(`${REPO_URL}/releases`)}
          className="flex items-center gap-3 w-full text-left px-3 py-2 rounded-md border border-gray-200 hover:border-blue-400 hover:bg-blue-50 transition-colors"
        >
          <Download className="w-4 h-4 text-gray-700" />
          <span className="text-sm text-gray-800">{t('viewReleases')}</span>
        </button>
        <button
          onClick={() => openUrl(`${REPO_URL}/blob/main/PRIVACY_POLICY.md`)}
          className="flex items-center gap-3 w-full text-left px-3 py-2 rounded-md border border-gray-200 hover:border-blue-400 hover:bg-blue-50 transition-colors"
        >
          <Shield className="w-4 h-4 text-gray-700" />
          <span className="text-sm text-gray-800">{t('privacyPolicy')}</span>
        </button>
        {/* The same notices are installed in `licenses\` next to the app. */}
        <button
          onClick={() => openUrl(`${REPO_URL}/blob/main/frontend/src-tauri/licenses/THIRD-PARTY-NOTICES.md`)}
          className="flex items-center gap-3 w-full text-left px-3 py-2 rounded-md border border-gray-200 hover:border-blue-400 hover:bg-blue-50 transition-colors"
        >
          <FileText className="w-4 h-4 text-gray-700" />
          <span className="text-sm text-gray-800">{t('thirdPartyLicenses')}</span>
        </button>
      </div>

      <div className="text-center text-xs text-gray-400">
        <p>
          {t.rich('developerCredit', {
            handle: (chunks) => (
              <button
                type="button"
                onClick={() => openUrl(PROFILE_URL)}
                title={t('profileOnGithub')}
                className="text-blue-500 hover:underline"
              >
                {chunks}
              </button>
            ),
          })}
          {' · '}
          {t('footerLicense')}
        </p>
      </div>
    </div>
  );
}

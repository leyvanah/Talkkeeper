import React, { useEffect, useState } from 'react';
import { useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import { Lock, Sparkles, Cpu, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding } from '@/contexts/OnboardingContext';
import { usePlatform } from '@/hooks/usePlatform';
import { UPDATES_AVAILABLE } from '@/lib/updates';

export function WelcomeStep() {
  const t = useTranslations('onboarding');
  const { goNext } = useOnboarding();
  const platform = usePlatform();
  const updatesSupported = UPDATES_AVAILABLE && platform !== 'macos';
  const [checkUpdates, setCheckUpdates] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);

  const features = [
    {
      icon: Lock,
      title: t('featurePrivacy'),
    },
    {
      icon: Sparkles,
      title: t('featureSummaries'),
    },
    {
      icon: Cpu,
      title: t('featureOffline'),
    },
  ];

  useEffect(() => {
    if (!updatesSupported) setCheckUpdates(false);
  }, [updatesSupported]);

  const continueOnboarding = async () => {
    if (checkUpdates === null || saving) return;
    setSaving(true);
    try {
      await invoke('set_check_updates_on_launch', { enabled: checkUpdates });
      goNext();
    } catch (error) {
      console.error('Failed to save update preference:', error);
      setSaving(false);
    }
  };

  return (
    <OnboardingContainer
      title={t('welcomeTitle')}
      description={t('welcomeDescription')}
      step={1}
      hideProgress={true}
    >
      <div className="flex flex-col items-center space-y-6">
        {/* Divider */}
        <div className="w-16 h-px bg-gray-300" />

        {/* Features Card */}
        <div className="w-full max-w-md bg-white rounded-lg border border-gray-200 shadow-sm p-6 space-y-4">
          {features.map((feature, index) => {
            const Icon = feature.icon;
            return (
              <div key={index} className="flex items-start gap-3">
                <div className="flex-shrink-0 mt-0.5">
                  <div className="w-5 h-5 rounded-full bg-gray-100 flex items-center justify-center">
                    <Icon className="w-3 h-3 text-gray-700" />
                  </div>
                </div>
                <p className="text-sm text-gray-700 leading-relaxed">{feature.title}</p>
              </div>
            );
          })}
        </div>

        {updatesSupported && <div className="w-full max-w-md rounded-lg border border-gray-200 bg-white p-5 shadow-sm">
          <div className="mb-4 flex items-start gap-3">
            <div className="mt-0.5 flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-full bg-gray-100">
              <RefreshCw className="h-3.5 w-3.5 text-gray-700" />
            </div>
            <div>
              <h2 className="text-sm font-medium text-gray-900">{t('updatesQuestion')}</h2>
              <p className="mt-1 text-xs leading-relaxed text-gray-500">
                {t('updatesNote')}
              </p>
            </div>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <button
              type="button"
              onClick={() => setCheckUpdates(true)}
              aria-pressed={checkUpdates === true}
              className={`rounded-lg border px-3 py-2 text-sm transition-colors ${
                checkUpdates === true
                  ? 'border-gray-900 bg-gray-900 text-white'
                  : 'border-gray-200 text-gray-700 hover:border-gray-400'
              }`}
            >
              {t('updatesYes')}
            </button>
            <button
              type="button"
              onClick={() => setCheckUpdates(false)}
              aria-pressed={checkUpdates === false}
              className={`rounded-lg border px-3 py-2 text-sm transition-colors ${
                checkUpdates === false
                  ? 'border-gray-900 bg-gray-900 text-white'
                  : 'border-gray-200 text-gray-700 hover:border-gray-400'
              }`}
            >
              {t('updatesNo')}
            </button>
          </div>
        </div>}

        {/* CTA Section */}
        <div className="w-full max-w-xs space-y-3">
          {checkUpdates !== null && (
            <Button
              onClick={() => void continueOnboarding()}
              disabled={saving}
              className="w-full h-11 bg-gray-900 hover:bg-gray-800 text-white"
            >
              {saving ? t('saving') : t('getStarted')}
            </Button>
          )}
          <p className="text-xs text-center text-gray-500">{t('takesLessThan')}</p>
        </div>
      </div>
    </OnboardingContainer>
  );
}

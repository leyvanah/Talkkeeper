import React, { useEffect, useState } from 'react';
import { useLocale, useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import { Cpu, Info, Zap } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  formatWhisperBackend,
  getWhisperBackend,
  type TranscriptionAccelerationStatus,
  type WhisperBackend,
} from '@/lib/transcription-acceleration';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding, type SttEngine } from '@/contexts/OnboardingContext';
import { onboardingDownloadMb } from '@/lib/onboarding-summary-model';
import { formatDiskSize } from '@/lib/disk-space';
import { useDiskSpaceText } from '@/hooks/useDiskSpaceText';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';

export function SetupOverviewStep() {
  const t = useTranslations('onboarding');
  const locale = useLocale();
  const diskSpaceText = useDiskSpaceText();
  const {
    goNext,
    sttEngine,
    setSttEngine,
    sttDownloaded,
    selectedSummaryModel,
    recommendedSummaryModel,
    summaryModelDownloaded,
  } = useOnboarding();
  const [spaceError, setSpaceError] = useState<string | null>(null);
  const [checkingSpace, setCheckingSpace] = useState(false);
  // Both downloads, before the first one starts: recognition fetched only
  // for the summary model to run out of space halfway helps nobody.
  const downloadBytes =
    onboardingDownloadMb({
      sttEngine,
      sttDownloaded,
      summaryModel: selectedSummaryModel || recommendedSummaryModel,
      summaryDownloaded: summaryModelDownloaded,
    }) * 1024 * 1024;
  const [isMac, setIsMac] = useState(false);
  const [whisperBackend, setWhisperBackend] = useState<WhisperBackend | null | undefined>();

  // Which recognition backend this build has. There is one build per
  // release, so this is a fact to show, not something to change here.
  const checkAcceleration = async () => {
    try {
      const status = await invoke<TranscriptionAccelerationStatus>('get_local_stack_status');
      setWhisperBackend(getWhisperBackend(status));
    } catch (error) {
      console.error('Failed to detect transcription acceleration:', error);
      setWhisperBackend(null);
    }
  };

  useEffect(() => {
    const checkPlatform = async () => {
      try {
        const { platform } = await import('@tauri-apps/plugin-os');
        setIsMac(platform() === 'macos');
      } catch (e) {
        setIsMac(navigator.userAgent.includes('Mac'));
      }
    };
    checkPlatform();
    void checkAcceleration();
  }, []);

  const engines: { id: SttEngine; title: string; note: string; badge?: string }[] = [
    {
      id: 'gigaam',
      title: 'GigaAM v3',
      note: t('setupEngineGigaamNote'),
      badge: t('setupEngineGigaamBadge'),
    },
    {
      id: 'parakeet',
      title: 'Parakeet v3',
      note: t('setupEngineParakeetNote'),
    },
  ];

  const handleContinue = async () => {
    setSpaceError(null);
    if (downloadBytes > 0) {
      setCheckingSpace(true);
      try {
        await invoke('check_model_download_space', { downloadBytes });
      } catch (error) {
        const text = diskSpaceText(error);
        if (text) {
          setSpaceError(text);
          return;
        }
        // Not being able to tell is no reason to stop: each download checks
        // again before its first byte.
        console.error('Could not check free disk space:', error);
      } finally {
        setCheckingSpace(false);
      }
    }
    goNext();
  };

  const openIssues = () => {
    invoke('open_external_url', {
      url: 'https://github.com/leyvanah/Talkkeeper',
    }).catch((error) => console.error('Failed to open GitHub issues:', error));
  };

  const accelerationLabel = whisperBackend === undefined
    ? t('setupAccelerationDetecting')
    : whisperBackend === null
      ? t('setupAccelerationUnknown')
      : t('setupBackendSelected', { backend: formatWhisperBackend(whisperBackend) });
  const accelerationDescription = whisperBackend === undefined
    ? t('setupAccelerationChecking')
    : whisperBackend === null
      ? t('setupAccelerationReviewLater')
      : whisperBackend === 'CPU'
        ? t('setupAccelerationCpu')
        : t('setupAccelerationAuto');

  return (
    <OnboardingContainer
      title={t('setupTitle')}
      description={t('setupDescription')}
      step={2}
      totalSteps={isMac ? 4 : 3}
    >
      <div className="flex flex-col items-center space-y-10">
        {/* Steps Card */}
        <div className="w-full max-w-md bg-white rounded-lg border border-gray-200 p-4 space-y-5">
          <div className="space-y-3">
            <h3 className="font-medium text-gray-900 ml-1">
              {t('setupStepPrefix', { number: 1 })}  {t('setupStepTranscription')}
            </h3>
            <div role="radiogroup" aria-label={t('setupStepTranscription')} className="grid gap-2">
              {engines.map((engine) => {
                const selected = sttEngine === engine.id;
                return (
                  <button
                    key={engine.id}
                    type="button"
                    role="radio"
                    aria-checked={selected}
                    onClick={() => setSttEngine(engine.id)}
                    className={`rounded-lg border px-3 py-2.5 text-left transition-colors ${
                      selected
                        ? 'border-[var(--af-accent)] bg-[var(--af-accent-soft)] ring-1 ring-[var(--af-accent)]'
                        : 'border-gray-200 hover:border-gray-400'
                    }`}
                  >
                    <span className="flex items-center gap-2 text-sm font-medium text-gray-900">
                      {engine.title}
                      {engine.badge && (
                        <span
                          className="rounded-full bg-sky-500/10 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-sky-500"
                        >
                          {engine.badge}
                        </span>
                      )}
                    </span>
                    <span className="mt-0.5 block text-xs leading-5 text-gray-500">
                      {engine.note}
                    </span>
                  </button>
                );
              })}
            </div>
            <p className="ml-1 text-xs text-gray-500">{t('setupEngineChangeLater')}</p>
          </div>

          <h3 className="font-medium text-gray-900 flex items-center gap-2 ml-1">
            {t('setupStepPrefix', { number: 2 })}  {t('setupStepSummarization')}
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <button className="text-gray-400 hover:text-gray-600">
                    <Info className="w-4 h-4" />
                  </button>
                </TooltipTrigger>
                <TooltipContent className="max-w-xs text-sm">
                  {t('setupSummaryProvidersTooltip')}
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          </h3>
        </div>

        <div className="w-full max-w-md rounded-lg border border-gray-200 bg-white p-4">
          <div className="flex items-start gap-3">
            <div className="rounded-full bg-blue-50 p-2 text-blue-600">
              {whisperBackend === 'CPU' ? <Cpu className="h-4 w-4" /> : <Zap className="h-4 w-4" />}
            </div>
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-gray-900">{t('setupAcceleration')}</p>
              <p className="mt-1 text-sm font-semibold text-blue-700">
                {accelerationLabel}
              </p>
              <p className="mt-1 text-xs leading-5 text-gray-600">
                {accelerationDescription} {t('setupParakeetCpuNote')}
              </p>
            </div>
          </div>
        </div>

        {/* CTA Section */}
        <div className="w-full max-w-xs space-y-4">
          {downloadBytes > 0 && (
            <p className="text-center text-xs text-gray-500">
              {t('setupDownloadTotal', { size: formatDiskSize(downloadBytes, locale) })}
            </p>
          )}
          {spaceError && (
            <p role="alert" className="text-center text-sm text-red-600">
              {spaceError}
            </p>
          )}
          <Button
            onClick={() => void handleContinue()}
            disabled={checkingSpace}
            className="w-full h-11 bg-gray-900 hover:bg-gray-800 text-white"
          >
            {t('setupLetsGo')}
          </Button>
          <div className="text-center">
            <button
              type="button"
              onClick={openIssues}
              className="text-xs text-gray-600 hover:underline"
            >
              {t('setupViewOnGithub')}
            </button>
          </div>
        </div>
      </div>
    </OnboardingContainer>
  );
}

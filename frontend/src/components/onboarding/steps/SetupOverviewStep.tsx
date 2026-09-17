import React, { useEffect, useState } from 'react';
import { useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import { AlertTriangle, Cpu, Info, RefreshCw, Zap } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  formatWhisperBackend,
  getWhisperBackend,
  type CudaReconfigurationStatus,
  type TranscriptionAccelerationStatus,
  type WhisperBackend,
} from '@/lib/transcription-acceleration';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding } from '@/contexts/OnboardingContext';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';

export function SetupOverviewStep() {
  const t = useTranslations('onboarding');
  const { goNext } = useOnboarding();
  const [isMac, setIsMac] = useState(false);
  const [whisperBackend, setWhisperBackend] = useState<WhisperBackend | null | undefined>();
  const [cudaStatus, setCudaStatus] = useState<CudaReconfigurationStatus | null>(null);
  const [checkingAcceleration, setCheckingAcceleration] = useState(false);

  const checkAcceleration = async () => {
    setCheckingAcceleration(true);
    try {
      const [status, cuda] = await Promise.all([
        invoke<TranscriptionAccelerationStatus>('get_local_stack_status'),
        invoke<CudaReconfigurationStatus>('get_cuda_reconfiguration_status'),
      ]);
      setWhisperBackend(getWhisperBackend(status));
      setCudaStatus(cuda);
    } catch (error) {
      console.error('Failed to detect transcription acceleration:', error);
      setWhisperBackend(null);
      setCudaStatus(null);
    } finally {
      setCheckingAcceleration(false);
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

  const steps = [
    {
      number: 1,
      type: 'transcription',
      title: t('setupStepTranscription'),
    },
    {
      number: 2,
      type: 'summarization',
      title: t('setupStepSummarization'),
    },
  ];

  const handleContinue = () => {
    goNext();
  };

  const openIssues = () => {
    invoke('open_external_url', {
      url: 'https://github.com/leyvanah/Talkkeeper',
    }).catch((error) => console.error('Failed to open GitHub issues:', error));
  };

  const openNvidiaDrivers = () => {
    invoke('open_external_url', {
      url: 'https://www.nvidia.com/Download/index.aspx',
    }).catch((error) => console.error('Failed to open NVIDIA drivers:', error));
  };

  const openLatestSetup = () => {
    if (!cudaStatus?.setupDownloadUrl) return;
    invoke('open_external_url', {
      url: cudaStatus.setupDownloadUrl,
    }).catch((error) => console.error('Failed to open latest Talkkeeper setup:', error));
  };

  const accelerationLabel = cudaStatus?.reconfigurationRequired
    ? t('setupCudaAvailable')
    : whisperBackend === undefined
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
  const showCudaNotice = cudaStatus?.driverUpdateRequired || cudaStatus?.reconfigurationRequired;
  const cudaProbeFailed = cudaStatus?.driverState === 'query-failed';
  const cudaBuildInstalled = cudaStatus?.compiledBackend.toLowerCase() === 'cuda';

  return (
    <OnboardingContainer
      title={t('setupTitle')}
      description={t('setupDescription')}
      step={2}
      totalSteps={isMac ? 4 : 3}
    >
      <div className="flex flex-col items-center space-y-10">
        {/* Steps Card */}
        <div className="w-full max-w-md bg-white rounded-lg border border-gray-200 p-4">
          <div className="space-y-4">
            {steps.map((step) => {
              return (
                <div
                  key={step.number}
                  className="flex items-start gap-4 p-1"
                >
                  <div className="flex-1 ml-1">
                    <h3 className="font-medium text-gray-900 flex items-center gap-2">
                        {t('setupStepPrefix', { number: step.number })}  {step.title}

                        {step.type === 'summarization' && (
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
                        )}
                        </h3>
                  </div>
                </div>
              );
            })}
          </div>
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

        {showCudaNotice && (
          <div
            role="status"
            className="w-full max-w-md rounded-lg border border-amber-300 bg-amber-50 p-4 text-amber-950"
          >
            <div className="flex items-start gap-3">
              <AlertTriangle className="mt-0.5 h-5 w-5 shrink-0 text-amber-600" />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-semibold">
                  {cudaStatus?.reconfigurationRequired
                    ? t('setupCudaReady')
                    : cudaProbeFailed
                      ? t('setupCudaUnverified')
                      : t('setupCudaDriverNeeded')}
                </p>
                <p className="mt-1 text-xs leading-5 text-amber-900">
                  {cudaStatus?.reconfigurationRequired
                    ? t('setupCudaRerunSetup', { backend: formatWhisperBackend(whisperBackend ?? 'CPU') })
                    : cudaProbeFailed
                      ? t('setupCudaProbeFailed', { suffix: cudaBuildInstalled ? '' : t('setupCudaProbeFailedSuffix') })
                      : cudaBuildInstalled
                        ? t('setupCudaBuildNoDriver')
                        : t('setupCudaNoDriver')}
                </p>
                <div className="mt-3 flex flex-wrap gap-2">
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={cudaStatus?.reconfigurationRequired ? openLatestSetup : openNvidiaDrivers}
                    className="border-amber-400 bg-white text-amber-950 hover:bg-amber-100"
                  >
                    {cudaStatus?.reconfigurationRequired ? t('setupDownloadCuda') : t('setupGetNvidiaDriver')}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    disabled={checkingAcceleration}
                    onClick={() => void checkAcceleration()}
                    className="text-amber-950 hover:bg-amber-100"
                  >
                    <RefreshCw className={`mr-1.5 h-3.5 w-3.5 ${checkingAcceleration ? 'animate-spin' : ''}`} />
                    {t('setupRecheck')}
                  </Button>
                </div>
              </div>
            </div>
          </div>
        )}

        {/* CTA Section */}
        <div className="w-full max-w-xs space-y-4">
          <Button
            onClick={handleContinue}
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

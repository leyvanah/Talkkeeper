import React from 'react';
import { AlertTriangle, Mic, Speaker, RefreshCw } from 'lucide-react';
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert';
import { invoke } from '@tauri-apps/api/core';
import { usePlatform } from '@/hooks/usePlatform';
import { useTranslations } from 'next-intl';
import { permissionHelpFor, permissionKey, type MicrophoneProblem } from '@/lib/microphone-access';

interface PermissionWarningProps {
  hasMicrophone: boolean;
  microphoneProblem: MicrophoneProblem;
  hasSystemAudio: boolean;
  onRecheck: () => void;
  isRechecking?: boolean;
}

export function PermissionWarning({
  hasMicrophone,
  microphoneProblem,
  hasSystemAudio,
  onRecheck,
  isRechecking = false
}: PermissionWarningProps) {
  const t = useTranslations('recording');
  const platform = usePlatform();
  const isLinux = platform === 'linux';
  const help = permissionHelpFor(platform);

  // Don't show on Linux - permission handling is not needed
  if (isLinux) {
    return null;
  }

  // Don't show if both permissions are granted
  if (hasMicrophone && hasSystemAudio) {
    return null;
  }

  const isMacOS = help === 'macos';
  const isWindows = help === 'windows';

  const openMicrophoneSettings = async () => {
    if (isMacOS) {
      try {
        await invoke('open_system_settings', { preferencePane: 'Privacy_Microphone' });
      } catch (error) {
        console.error('Failed to open microphone settings:', error);
      }
    }
  };

  // Settings → Privacy & security → Microphone; a fixed ms-settings: page.
  const openWindowsMicrophoneSettings = async () => {
    try {
      await invoke('open_microphone_privacy_settings');
    } catch (error) {
      console.error('Failed to open Windows microphone settings:', error);
    }
  };

  const openAudioCaptureSettings = async () => {
    if (isMacOS) {
      try {
        await invoke('open_system_settings', { preferencePane: 'Privacy_AudioCapture' });
      } catch (error) {
        console.error('Failed to open Audio Capture settings:', error);
      }
    }
  };

  return (
    <div className="max-w-md mb-4 space-y-3">
      {/* Combined Permission Warning - Show when either permission is missing */}
      {(!hasMicrophone || !hasSystemAudio) && (
        <Alert variant="destructive" className="border-amber-400 bg-amber-50">
          <AlertTriangle className="h-5 w-5 text-amber-600" />
          <AlertTitle className="text-amber-900 font-semibold">
            <div className="flex items-center gap-2">
              {!hasMicrophone && <Mic className="h-4 w-4" />}
              {!hasSystemAudio && <Speaker className="h-4 w-4" />}
              {!hasMicrophone && !hasSystemAudio ? t('permissionsRequiredTitle') : !hasMicrophone ? t('microphonePermissionRequiredTitle') : t('systemAudioPermissionRequiredTitle')}
            </div>
          </AlertTitle>
          {/* Action Buttons */}
          <div className="mt-4 flex flex-wrap gap-2">
            {isMacOS && !hasMicrophone && (
              <button
                onClick={openMicrophoneSettings}
                className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-white bg-amber-600 hover:bg-amber-700 rounded-md transition-colors"
              >
                <Mic className="h-4 w-4" />
                {t('openMicrophoneSettings')}
              </button>
            )}
            {isWindows && !hasMicrophone && (
              <button
                onClick={openWindowsMicrophoneSettings}
                className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-white bg-amber-600 hover:bg-amber-700 rounded-md transition-colors"
              >
                <Mic className="h-4 w-4" />
                {t('openWindowsMicrophoneSettings')}
              </button>
            )}
            {isMacOS && !hasSystemAudio && (
              <button
                onClick={openAudioCaptureSettings}
                className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-white bg-blue-600 hover:bg-blue-700 rounded-md transition-colors"
              >
                <Speaker className="h-4 w-4" />
                {t('openAudioCaptureSettings')}
              </button>
            )}
            <button
              onClick={onRecheck}
              disabled={isRechecking}
              className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-amber-900 bg-amber-100 hover:bg-amber-200 rounded-md transition-colors disabled:opacity-50"
            >
              <RefreshCw className={`h-4 w-4 ${isRechecking ? 'animate-spin' : ''}`} />
              {t('recheck')}
            </button>
          </div>
          <AlertDescription className="text-amber-800 mt-2">
            {/* Microphone Warning */}
            {!hasMicrophone && microphoneProblem === 'blocked' && (
              <p className="mb-3">{t('microphoneBlockedWindows')}</p>
            )}
            {!hasMicrophone && microphoneProblem !== 'blocked' && (
              <>
                <p className="mb-3">
                  {t('microphoneAccessNeeded')}
                </p>
                <div className="space-y-2 text-sm mb-4">
                  <p className="font-medium">{t('pleaseCheck')}</p>
                  <ul className="list-disc list-inside ml-2 space-y-1">
                    <li>{t('micConnectedCheck')}</li>
                    <li>{t(permissionKey('micPermissionGrantedCheck', help, ['windows', 'macos']))}</li>
                    <li>{t('micNotInUseCheck')}</li>
                  </ul>
                </div>
              </>
            )}

            {/* System Audio Warning */}
            {!hasSystemAudio && (
              <>
                <p className="mb-3">
                  {hasMicrophone
                    ? t('systemAudioUnavailableWithMic')
                    : t('systemAudioUnavailableNoMic')}
                </p>
                {isMacOS && (
                  <div className="space-y-2 text-sm mb-4">
                    <p className="font-medium">{t('enableSystemAudioMacosTitle')}</p>
                    <ul className="list-disc list-inside ml-2 space-y-1">
                      <li>{t('grantAudioCapturePermission')}</li>
                      <li>{t('playAudioRecheckTap')}</li>
                      <li>{t('restartIfSilent')}</li>
                    </ul>
                  </div>
                )}
              </>
            )}


          </AlertDescription>
        </Alert>
      )}
    </div>
  );
}

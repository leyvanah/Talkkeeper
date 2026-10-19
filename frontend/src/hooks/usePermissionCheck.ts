import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { usePlatform } from './usePlatform';
import { microphoneProblem, type MicrophoneProblem } from '@/lib/microphone-access';

export interface PermissionStatus {
  hasMicrophone: boolean;
  /** Why there is no usable microphone; null when there is one. */
  microphoneProblem: MicrophoneProblem;
  hasSystemAudio: boolean;
  isChecking: boolean;
  error: string | null;
}

// Audible samples are the only reliable proof that the tap works. Keep this
// session-scoped so a permission revoked between app launches is not trusted.
export const MACOS_SYSTEM_AUDIO_VERIFIED_KEY = 'macos_system_audio_verified';

export function usePermissionCheck() {
  const platform = usePlatform();
  const requestInFlight = useRef<Promise<void> | null>(null);
  const [status, setStatus] = useState<PermissionStatus>({
    hasMicrophone: false,
    microphoneProblem: null,
    hasSystemAudio: false,
    isChecking: true,
    error: null,
  });

  const checkPermissions = async () => {
    setStatus(prev => ({ ...prev, isChecking: true, error: null }));

    try {
      // Get audio devices to check for microphone and system audio availability
      const devices = await invoke<Array<{ name: string; device_type: 'Input' | 'Output' }>>('get_audio_devices');

      // Check for microphone devices (Input). Windows keeps listing them when
      // its privacy settings deny the app the microphone, so ask about that too.
      const inputDevices = devices.filter(d => d.device_type === 'Input');
      const blockedBySystem = await invoke<boolean>('microphone_access_blocked').catch((error) => {
        console.warn('Could not read the microphone privacy setting:', error);
        return false;
      });
      const problem = microphoneProblem(inputDevices.length, blockedBySystem);
      const hasMicrophone = problem === null;

      // Output availability is separate from macOS Audio Capture authorization;
      // requestPermissions runs the native tap probe when the user asks.
      const outputDevices = devices.filter(d => d.device_type === 'Output');
      const systemAudioVerified =
        platform !== 'macos' ||
        window.sessionStorage.getItem(MACOS_SYSTEM_AUDIO_VERIFIED_KEY) === 'true';
      const hasSystemAudio = outputDevices.length > 0 && systemAudioVerified;

      console.log('Permission check:', {
        hasMicrophone,
        microphoneProblem: problem,
        hasSystemAudio,
        inputDevices: inputDevices.length,
        outputDevices: outputDevices.length
      });

      setStatus({
        hasMicrophone,
        microphoneProblem: problem,
        hasSystemAudio,
        isChecking: false,
        error: null,
      });

      return { hasMicrophone, hasSystemAudio };
    } catch (error) {
      console.error('Failed to check audio permissions:', error);
      setStatus({
        hasMicrophone: false,
        microphoneProblem: 'missing',
        hasSystemAudio: false,
        isChecking: false,
        error: error instanceof Error ? error.message : 'Failed to check permissions',
      });
      return { hasMicrophone: false, hasSystemAudio: false };
    }
  };

  const requestPermissions = () => {
    // The native probe runs for up to five seconds; deduplicate Recheck clicks
    // so an older result cannot overwrite a newer permission attempt.
    if (requestInFlight.current) return requestInFlight.current;

    const request = (async () => {
      setStatus(prev => ({ ...prev, isChecking: true, error: null }));
      try {
        await invoke('trigger_microphone_permission');
        let systemAudioDetected: boolean | null = null;
        if (platform === 'macos') {
          systemAudioDetected = await invoke<boolean>('trigger_system_audio_permission_command');
          window.sessionStorage.setItem(
            MACOS_SYSTEM_AUDIO_VERIFIED_KEY,
            String(systemAudioDetected),
          );
        }

        await new Promise(resolve => setTimeout(resolve, 1000));
        const availability = await checkPermissions();
        if (systemAudioDetected !== null) {
          setStatus(prev => ({
            ...prev,
            hasSystemAudio: availability.hasSystemAudio && systemAudioDetected,
          }));
        }
      } catch (error) {
        console.error('Failed to request permissions:', error);
        if (platform === 'macos') {
          window.sessionStorage.setItem(MACOS_SYSTEM_AUDIO_VERIFIED_KEY, 'false');
        }
        setStatus(prev => ({
          ...prev,
          hasSystemAudio: false,
          error: error instanceof Error ? error.message : 'Failed to request permissions',
        }));
      } finally {
        requestInFlight.current = null;
        setStatus(prev => ({ ...prev, isChecking: false }));
      }
    })();

    requestInFlight.current = request;
    return request;
  };

  // Check permissions on mount
  useEffect(() => {
    checkPermissions();
  }, []);

  return {
    ...status,
    checkPermissions,
    requestPermissions,
  };
}

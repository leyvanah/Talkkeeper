import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { configService } from '@/services/configService';

/** How often to look again while the model is still missing. */
const RECHECK_MS = 30_000;

/**
 * Whether the speech recognition model the settings have chosen is missing.
 *
 * Onboarding lets the owner through without it when the download failed -
 * no network, most likely (tasks#73) - and the main screen says so until the
 * model is there. A check that fails says "not missing": better no banner
 * than a wrong one.
 */
export function useSpeechModelMissing(): boolean {
  const [missing, setMissing] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const check = async () => {
      const isMissing = await speechModelMissing();
      if (cancelled) return;
      setMissing(isMissing);
      if (isMissing) timer = setTimeout(() => void check(), RECHECK_MS);
    };
    void check();

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, []);

  return missing;
}

async function speechModelMissing(): Promise<boolean> {
  try {
    const config = await configService.getTranscriptConfig().catch(() => null);
    const provider = (config?.provider || 'parakeet').toLowerCase();
    if (provider === 'externalstt') return false;
    if (provider === 'gigaam') {
      const status = await invoke<{ installed: boolean }>('gigaam_get_model_status');
      return !status.installed;
    }
    if (provider.includes('whisper')) {
      await invoke('whisper_init');
      return !(await invoke<boolean>('whisper_has_available_models'));
    }
    await invoke('parakeet_init');
    return !(await invoke<boolean>('parakeet_has_available_models'));
  } catch (error) {
    console.warn('Could not check whether the speech model is installed:', error);
    return false;
  }
}

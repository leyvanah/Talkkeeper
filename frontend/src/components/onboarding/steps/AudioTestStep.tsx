'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useOnboarding } from '@/contexts/OnboardingContext';
import { usePlatform } from '@/hooks/usePlatform';
import { MACOS_SYSTEM_AUDIO_VERIFIED_KEY } from '@/hooks/usePermissionCheck';
import { logFailure, toastFailure } from '@/lib/failure';
import { OnboardingContainer } from '../OnboardingContainer';
import { Mic, Volume2, RefreshCw } from 'lucide-react';

interface AudioDevice {
  name: string;
  device_type: 'Input' | 'Output' | string;
}

interface AudioLevelData {
  device_name: string;
  device_type: string;
  rms_level: number;
  peak_level: number;
  is_active: boolean;
}

interface AudioLevelUpdate {
  timestamp: number;
  levels: AudioLevelData[];
}

/** `get_default_audio_devices`: the system's defaults, by their listed names. */
interface DefaultDevices {
  input: string | null;
  output: string | null;
}

/** The default device when it is in the list, else the first one. */
function pickDevice(list: AudioDevice[], preferred: string | null): string {
  return (preferred && list.find((d) => d.name === preferred)?.name) || list[0]?.name || '';
}

/**
 * Quick mic + system-audio level check so users know capture works before
 * their first real meeting.
 */
export function AudioTestStep() {
  const t = useTranslations('onboarding');
  const { goPrevious, completeOnboarding } = useOnboarding();
  const platform = usePlatform();
  const isMacOS = platform === 'macos';
  const [micRms, setMicRms] = useState(0);
  const [sysRms, setSysRms] = useState(0);
  const [micHeard, setMicHeard] = useState(false);
  const [sysHeard, setSysHeard] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState(() => t('audioStartingMeters'));
  const [inputs, setInputs] = useState<AudioDevice[]>([]);
  const [outputs, setOutputs] = useState<AudioDevice[]>([]);
  const [micName, setMicName] = useState<string>('');
  const [sysName, setSysName] = useState<string>('');
  const [defaultMic, setDefaultMic] = useState<string | null>(null);
  const [defaultSys, setDefaultSys] = useState<string | null>(null);
  const [defaultsKnown, setDefaultsKnown] = useState(false);
  // Whether the owner picked a device from the list, rather than keeping
  // the one it opened on.
  const micChosen = useRef(false);
  const sysChosen = useRef(false);
  const monitoring = useRef(false);
  const active = useRef(true);
  const deviceLoad = useRef(0);
  const meterRun = useRef(0);
  // Rust owns one process-global monitor. Serialize stop/start transitions so a
  // stale StrictMode effect, retest, or device change cannot stop its successor.
  const meterTransition = useRef<Promise<void>>(Promise.resolve());
  const micNameRef = useRef('');
  const sysNameRef = useRef('');

  const stop = useCallback(async () => {
    if (!monitoring.current) return;
    monitoring.current = false;
    try {
      await invoke('stop_audio_level_monitoring');
    } catch {
      /* ignore */
    }
  }, []);

  const startMeters = useCallback(
    (mic: string, sys: string) => {
      const run = ++meterRun.current;
      const transition = meterTransition.current.then(async () => {
        await stop();
        if (!active.current || run !== meterRun.current) return;
        setError(null);
        setMicRms(0);
        setSysRms(0);
        setStatus(t('audioOpeningDevices'));

        const deviceNames = (isMacOS ? [mic] : [mic, sys]).filter(
          (name) => name && name.trim().length > 0,
        );
        if (!mic && !sys) {
          setError(t('audioNoDevicesFound'));
          setStatus(t('audioNoDevices'));
          return;
        }

        try {
          // Ask the OS for mic permission before opening streams.
          try {
            await invoke('trigger_microphone_permission');
          } catch {
            /* non-fatal */
          }
          if (!active.current || run !== meterRun.current) return;

          if (isMacOS && sys) {
            setStatus(t('audioTestingNative'));
            try {
              const detected = await invoke<boolean>('trigger_system_audio_permission_command');
              if (!active.current || run !== meterRun.current) return;
              window.sessionStorage.setItem(MACOS_SYSTEM_AUDIO_VERIFIED_KEY, String(detected));
              setSysHeard(detected);
              setSysRms(detected ? 0.2 : 0);
              if (!detected) {
                setError(
                  t('audioSystemNotDetected'),
                );
              }
            } catch (systemError) {
              if (!active.current || run !== meterRun.current) return;
              window.sessionStorage.setItem(MACOS_SYSTEM_AUDIO_VERIFIED_KEY, 'false');
              logFailure('onboarding-system-audio-test', systemError);
              setError(t('audioSystemTestFailed'));
            }
          }
          if (!active.current || run !== meterRun.current) return;

          micNameRef.current = mic;
          sysNameRef.current = sys;
          if (deviceNames.length > 0) {
            monitoring.current = true;
            await invoke('start_audio_level_monitoring', { deviceNames });
            if (!active.current || run !== meterRun.current) {
              await invoke('stop_audio_level_monitoring').catch(() => undefined);
              monitoring.current = false;
              return;
            }
          }
          setStatus(
            mic
              ? t(isMacOS ? 'audioListeningMac' : 'audioListening', { device: shortName(mic) })
              : t(isMacOS ? 'audioListeningNoMicMac' : 'audioListeningNoMic'),
          );
        } catch (e) {
          if (!active.current || run !== meterRun.current) return;
          monitoring.current = false;
          logFailure('onboarding-audio-meters', e);
          setError(t('audioMetersFailed'));
          setStatus(t('audioCheckFailed'));
        }
      });
      meterTransition.current = transition.catch(() => undefined);
      return transition;
    },
    [isMacOS, stop],
  );

  const queueStop = useCallback(() => {
    meterRun.current += 1;
    const transition = meterTransition.current.then(stop, stop);
    meterTransition.current = transition.catch(() => undefined);
    return transition;
  }, [stop]);

  const loadDevicesAndStart = useCallback(async () => {
    const run = ++deviceLoad.current;
    setError(null);
    setStatus(t('audioFindingDevices'));
    try {
      const [devices, readDefaults] = await Promise.all([
        invoke<AudioDevice[]>('get_audio_devices'),
        invoke<DefaultDevices>('get_default_audio_devices').catch((e) => {
          logFailure('onboarding-default-devices', e);
          return null;
        }),
      ]);
      if (!active.current || run !== deviceLoad.current) return;
      const defaults: DefaultDevices = readDefaults ?? { input: null, output: null };
      const inputList = devices.filter((d) => String(d.device_type).toLowerCase() === 'input');
      const outputList = devices.filter((d) => String(d.device_type).toLowerCase() === 'output');
      setInputs(inputList);
      setOutputs(outputList);
      setDefaultMic(defaults.input);
      setDefaultSys(defaults.output);
      setDefaultsKnown(readDefaults !== null);
      // A fresh list opens on the defaults again.
      micChosen.current = false;
      sysChosen.current = false;

      // The system's default first: it is what a recording uses unless the
      // recording settings name another, so the check listens to it.
      const nextMic = pickDevice(inputList, defaults.input);
      const nextSys = pickDevice(outputList, defaults.output);
      setMicName(nextMic);
      setSysName(nextSys);

      if (!nextMic && !nextSys) {
        setError(t('audioNoDevicesDetected'));
        setStatus(t('audioNoDevices'));
        return;
      }

      await startMeters(nextMic, nextSys);
    } catch (e) {
      if (!active.current || run !== deviceLoad.current) return;
      logFailure('onboarding-audio-devices', e);
      setError(t('audioListDevicesFailed'));
      setStatus(t('audioCheckFailed'));
    }
  }, [startMeters]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    active.current = true;

    (async () => {
      try {
        unlisten = await listen<AudioLevelUpdate>('audio-levels', (event) => {
          if (cancelled) return;
          const levels = event.payload?.levels || [];
          for (const level of levels) {
            const rms =
              typeof level.rms_level === 'number'
                ? level.rms_level
                : typeof (level as { peak_level?: number }).peak_level === 'number'
                  ? (level as { peak_level: number }).peak_level * 0.7
                  : 0;
            const kind = (level.device_type || '').toLowerCase();
            const name = level.device_name || '';

            const isMic =
              kind === 'input' ||
              kind.includes('mic') ||
              (!!micNameRef.current && name === micNameRef.current);
            const isSys =
              kind === 'output' ||
              kind.includes('system') ||
              (!!sysNameRef.current && name === sysNameRef.current && !isMic);

            if (isMic) {
              setMicRms(rms);
              if (rms > 0.008) setMicHeard(true);
            } else if (isSys) {
              setSysRms(rms);
              if (rms > 0.008) setSysHeard(true);
            }
          }
        });
      } catch (e) {
        console.error('audio-levels listen failed', e);
      }

      if (!cancelled) {
        await loadDevicesAndStart();
      }
    })();

    return () => {
      cancelled = true;
      active.current = false;
      deviceLoad.current += 1;
      unlisten?.();
      void queueStop();
    };
  }, [loadDevicesAndStart, queueStop]);

  const onMicChange = async (name: string) => {
    micChosen.current = true;
    setMicName(name);
    setMicHeard(false);
    await startMeters(name, sysName);
  };

  const onSysChange = async (name: string) => {
    sysChosen.current = true;
    setSysName(name);
    setSysHeard(false);
    await startMeters(micName, name);
  };

  const finish = async () => {
    await queueStop();
    // The devices checked here are the ones recordings use (decision of
    // 06.10). The system default is kept as "follow the system", not by its
    // name, so a recording still follows it when the default changes. When
    // the defaults could not be read, only a device the owner picked is kept
    // by name; the one the list opened on may not be the default at all.
    const keep = (name: string, systemDefault: string | null, chosen: boolean) => {
      if (!name) return null;
      if (!defaultsKnown) return chosen ? name : null;
      return name !== systemDefault ? name : null;
    };
    try {
      const preferences = await invoke<Record<string, unknown>>('get_recording_preferences');
      await invoke('set_recording_preferences', {
        preferences: {
          ...preferences,
          preferred_mic_device: keep(micName, defaultMic, micChosen.current),
          preferred_system_device: isMacOS ? null : keep(sysName, defaultSys, sysChosen.current),
        },
      });
    } catch (e) {
      // Not worth stopping onboarding for: Settings -> Recording has the same choice.
      logFailure('onboarding-save-devices', e);
    }
    try {
      await completeOnboarding();
      await new Promise((r) => setTimeout(r, 100));
      window.location.reload();
    } catch (e) {
      toastFailure(t('finishFailed'), 'onboarding-complete', e, {
        description: t('retryDetail'),
      });
    }
  };

  const bar = (rms: number, ok: boolean) => (
    <div className="h-2 w-full overflow-hidden rounded-full bg-[var(--af-panel-2)]">
      <div
        className={`h-full rounded-full transition-all duration-75 ${
          ok ? 'bg-emerald-500' : 'bg-[var(--af-accent)]'
        }`}
        style={{ width: `${Math.min(100, Math.round(Math.max(rms, 0) * 500))}%` }}
      />
    </div>
  );

  const selectClass =
    'w-full rounded-lg border border-[var(--af-border)] bg-[var(--af-panel-2)] px-3 py-2 text-sm text-[var(--af-text)] outline-none focus:border-[var(--af-accent)]';

  return (
    <OnboardingContainer
      title={t('audioTestTitle')}
      description={isMacOS ? t('audioTestDescriptionMac') : t('audioTestDescription')}
      step={5}
      showNavigation
      onPrevious={async () => {
        await queueStop();
        goPrevious();
      }}
      canGoPrevious
    >
      <div className="mx-auto max-w-md space-y-5">
        <div className="rounded-xl border border-[var(--af-border)] bg-[var(--af-panel)] p-4 space-y-3">
          <div className="flex items-center justify-between text-sm font-medium text-[var(--af-text)]">
            <span className="inline-flex items-center gap-2">
              <Mic size={16} className="text-blue-400" /> {t('audioMicrophone')}
            </span>
            <span className={micHeard ? 'text-emerald-400 text-xs' : 'text-[var(--af-text-3)] text-xs'}>
              {micHeard ? t('audioHeardYou') : t('audioSpeakNow')}
            </span>
          </div>
          {inputs.length > 0 ? (
            <select
              className={selectClass}
              value={micName}
              onChange={(e) => void onMicChange(e.target.value)}
            >
              {inputs.map((d) => (
                <option key={d.name} value={d.name}>
                  {d.name === defaultMic ? t('audioDeviceDefault', { device: d.name }) : d.name}
                </option>
              ))}
            </select>
          ) : (
            <p className="text-xs text-[var(--af-text-3)]">{t('audioNoMicrophones')}</p>
          )}
          {bar(micRms, micHeard)}
        </div>

        <div className="rounded-xl border border-[var(--af-border)] bg-[var(--af-panel)] p-4 space-y-3">
          <div className="flex items-center justify-between text-sm font-medium text-[var(--af-text)]">
            <span className="inline-flex items-center gap-2">
              <Volume2 size={16} className="text-purple-400" /> {t('audioSystemAudio')}
            </span>
            <span className={sysHeard ? 'text-emerald-400 text-xs' : 'text-[var(--af-text-3)] text-xs'}>
              {sysHeard ? t('audioDetected') : t('audioPlayVideo')}
            </span>
          </div>
          {isMacOS && outputs.length > 0 ? (
            <p className="text-xs text-[var(--af-text-3)]">
              {t('audioMacDefaultOutput')}
            </p>
          ) : outputs.length > 0 ? (
            <select
              className={selectClass}
              value={sysName}
              onChange={(e) => void onSysChange(e.target.value)}
            >
              {outputs.map((d) => (
                <option key={d.name} value={d.name}>
                  {d.name === defaultSys ? t('audioDeviceDefault', { device: d.name }) : d.name}
                </option>
              ))}
            </select>
          ) : (
            <p className="text-xs text-[var(--af-text-3)]">{t('audioNoPlaybackDevices')}</p>
          )}
          {bar(sysRms, sysHeard)}
        </div>

        <div className="flex items-center justify-between gap-3">
          <p className="text-xs text-[var(--af-text-3)]">{status}</p>
          <button
            type="button"
            onClick={() => void loadDevicesAndStart()}
            className="inline-flex items-center gap-1.5 rounded-lg border border-[var(--af-border)] px-2.5 py-1.5 text-xs text-[var(--af-text-2)] hover:bg-[var(--af-panel-2)]"
          >
            <RefreshCw size={12} /> {isMacOS ? t('audioRetest') : t('audioRefreshDevices')}
          </button>
        </div>

        {error && <p className="text-center text-xs text-amber-400 break-words">{error}</p>}
        <p className="text-center text-xs text-[var(--af-text-3)]">
          {t('audioFinishHint')} {t('audioChoiceSaved')}
        </p>

        <button
          type="button"
          onClick={() => void finish()}
          className="w-full h-11 rounded-xl bg-[var(--af-accent)] text-sm font-semibold text-[var(--af-accent-contrast)] shadow-sm transition hover:brightness-110 active:scale-[0.99]"
        >
          {micHeard || sysHeard ? t('continue') : t('audioSkipForNow')}
        </button>
      </div>
    </OnboardingContainer>
  );
}

function shortName(name: string): string {
  return name.length > 36 ? `${name.slice(0, 34)}…` : name;
}

export default AudioTestStep;

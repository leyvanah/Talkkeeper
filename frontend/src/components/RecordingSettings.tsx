import React, { useState, useEffect, useRef } from 'react';
import { useTranslations } from 'next-intl';
import { Switch } from '@/components/ui/switch';
import { FolderCog, FolderOpen } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { DeviceSelection, SelectedDevices } from '@/components/DeviceSelection';
import { toast } from 'sonner';
import { useConfig } from '@/contexts/ConfigContext';
import { toastFailure } from '@/lib/failure';
import { usePlatform } from '@/hooks/usePlatform';
import {
  lockedRecordingSwitch,
  shownEchoMode,
  type EchoMode,
} from '@/lib/recording-options';

export interface RecordingPreferences {
  save_folder: string;
  auto_save: boolean;
  file_format: string;
  preferred_mic_device: string | null;
  preferred_system_device: string | null;
  /** Extra mic loudness after normalize (0.5–3.0). */
  mic_gain?: number;
  /** System-audio gain before metering, transcription, and recording (0.5–3.0). */
  system_gain?: number;
  /** Drop microphone text that only repeats a recent system phrase. */
  echo_text_filter?: boolean;
  /** One conversation partner: microphone is you, the speakers are them. */
  single_remote_speaker?: boolean;
  /** How the speakers' echo is kept out of the microphone. The backend
   *  derives its older switches from this one. */
  echo_mode?: EchoMode;
  /** Only with the built-in canceller: drop microphone phrases heard while
   *  the speakers played and the owner was silent. */
  own_speech_detector?: boolean;
  /** Recognise speech while recording; off, a recording is only sound. */
  live_transcription?: boolean;
}

interface RecordingSettingsProps {
  onSave?: (preferences: RecordingPreferences) => void;
}

/** One way of handling the echo, as a choice in the echo card. */
function EchoModeOption({
  selected,
  disabled,
  onSelect,
  title,
  hint,
  unavailable,
}: {
  selected: boolean;
  disabled: boolean;
  onSelect: () => void;
  title: string;
  hint: string;
  /** Why the option cannot be chosen here; shown instead of acting. */
  unavailable?: string;
}) {
  const blocked = disabled || Boolean(unavailable);
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      aria-disabled={blocked}
      disabled={blocked}
      onClick={selected ? undefined : onSelect}
      // The theme recolours border utilities, so the chosen one is set here.
      style={{ borderColor: selected ? 'var(--af-accent)' : 'var(--af-border)' }}
      className={`flex w-full items-start gap-2.5 rounded-md border px-3 py-2 text-left transition-colors disabled:cursor-not-allowed ${
        unavailable ? 'opacity-60' : ''
      } ${selected ? 'bg-[var(--af-hover)]' : 'enabled:hover:bg-[var(--af-hover)]'}`}
    >
      <span
        aria-hidden
        className="mt-1 flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-full border"
        style={{ borderColor: selected ? 'var(--af-accent)' : 'var(--af-text-3)' }}
      >
        {selected && <span className="h-1.5 w-1.5 rounded-full bg-[var(--af-accent)]" />}
      </span>
      <span className="min-w-0">
        <span className="block text-sm font-medium text-[var(--af-text,#111827)]">{title}</span>
        <span className="mt-0.5 block text-xs text-gray-500">{unavailable ?? hint}</span>
      </span>
    </button>
  );
}

export function RecordingSettings({ onSave }: RecordingSettingsProps) {
  const t = useTranslations('settings');
  const { updateRecordingsLocation } = useConfig();
  const [preferences, setPreferences] = useState<RecordingPreferences>({
    save_folder: '',
    auto_save: true,
    file_format: 'mp4',
    preferred_mic_device: null,
    preferred_system_device: null,
    mic_gain: 1.0,
    system_gain: 1.0,
    echo_text_filter: false,
    single_remote_speaker: true,
    echo_mode: 'windows',
    own_speech_detector: false,
    live_transcription: true,
  });
  // What the settings hold: as loaded, then as last written. A failed save
  // puts the screen back to this.
  const savedRef = useRef(preferences);
  const isWindows = usePlatform() === 'windows';
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [isChoosingFolder, setIsChoosingFolder] = useState(false);
  const [showRecordingNotification, setShowRecordingNotification] = useState(true);

  // Load recording preferences on component mount
  useEffect(() => {
    const loadPreferences = async () => {
      try {
        const prefs = await invoke<RecordingPreferences>('get_recording_preferences');
        setPreferences(prefs);
        savedRef.current = prefs;
      } catch (error) {
        console.error('Failed to load recording preferences:', error);
        // If loading fails, get default folder path
        try {
          const defaultPath = await invoke<string>('get_default_recordings_folder_path');
          setPreferences(prev => ({ ...prev, save_folder: defaultPath }));
        } catch (defaultError) {
          console.error('Failed to get default folder path:', defaultError);
        }
      } finally {
        setLoading(false);
      }
    };

    loadPreferences();
  }, []);

  // Load recording notification preference
  useEffect(() => {
    const loadNotificationPref = async () => {
      try {
        const { Store } = await import('@tauri-apps/plugin-store');
        const store = await Store.load('preferences.json');
        const show = await store.get<boolean>('show_recording_notification') ?? true;
        setShowRecordingNotification(show);
      } catch (error) {
        console.error('Failed to load notification preference:', error);
      }
    };
    loadNotificationPref();
  }, []);

  const handleAutoSaveToggle = async (enabled: boolean) => {
    const newPreferences = { ...preferences, auto_save: enabled };
    setPreferences(newPreferences);
    await savePreferences(newPreferences, 'setting');
  };

  const handleEchoModeChange = async (echo_mode: EchoMode) => {
    const newPreferences = {
      ...preferences,
      echo_mode,
      // The detector belongs to the built-in canceller; the backend drops it
      // with any other choice, and so does the screen.
      own_speech_detector: echo_mode === 'builtin' ? preferences.own_speech_detector : false,
    };
    setPreferences(newPreferences);
    await savePreferences(newPreferences, 'setting');
  };

  const handleOwnSpeechDetectorToggle = async (enabled: boolean) => {
    const newPreferences = { ...preferences, own_speech_detector: enabled };
    setPreferences(newPreferences);
    await savePreferences(newPreferences, 'setting');
  };

  const handleLiveTranscriptionToggle = async (enabled: boolean) => {
    const newPreferences = { ...preferences, live_transcription: enabled };
    setPreferences(newPreferences);
    await savePreferences(newPreferences, 'setting');
  };

  const handleSingleRemoteSpeakerToggle = async (enabled: boolean) => {
    const newPreferences = { ...preferences, single_remote_speaker: enabled };
    setPreferences(newPreferences);
    await savePreferences(newPreferences, 'setting');
  };

  const handleEchoTextFilterToggle = async (enabled: boolean) => {
    const newPreferences = { ...preferences, echo_text_filter: enabled };
    setPreferences(newPreferences);
    await savePreferences(newPreferences, 'setting');
  };

  const handleMicGainChange = async (value: number) => {
    const mic_gain = Math.min(3, Math.max(0.5, value));
    // Letting go of the slider and leaving it both land here.
    if (mic_gain === (savedRef.current.mic_gain ?? 1)) return;
    const newPreferences = { ...preferences, mic_gain };
    setPreferences(newPreferences);
    await savePreferences(newPreferences, 'quiet');
  };

  const handleSystemGainChange = async (value: number) => {
    const system_gain = Math.min(3, Math.max(0.5, value));
    // Letting go of the slider and leaving it both land here.
    if (system_gain === (savedRef.current.system_gain ?? 1)) return;
    const newPreferences = { ...preferences, system_gain };
    setPreferences(newPreferences);
    await savePreferences(newPreferences, 'quiet');
  };

  const handleDeviceChange = async (devices: SelectedDevices) => {
    const newPreferences = {
      ...preferences,
      preferred_mic_device: devices.micDevice,
      preferred_system_device: devices.systemDevice
    };
    setPreferences(newPreferences);
    await savePreferences(newPreferences, 'devices');
  };

  const handleOpenFolder = async () => {
    try {
      await invoke('open_recordings_folder');
    } catch (error) {
      toastFailure(t('openRecordingsFolderFailed'), 'recordings-folder-open', error);
    }
  };

  const handleChangeFolder = async () => {
    if (isChoosingFolder) return;

    setIsChoosingFolder(true);
    try {
      const selectedFolder = await invoke<string | null>('select_recording_folder');
      if (!selectedFolder) return;

      const newPreferences = { ...preferences, save_folder: selectedFolder };
      await invoke('set_recording_preferences', { preferences: newPreferences });
      savedRef.current = newPreferences;
      setPreferences(newPreferences);
      updateRecordingsLocation(selectedFolder);
      onSave?.(newPreferences);
      toast.success(t('recordingsFolderUpdated'));
    } catch (error) {
      toastFailure(t('recordingsFolderUpdateFailed'), 'recordings-folder-change', error);
    } finally {
      setIsChoosingFolder(false);
    }
  };

  const handleNotificationToggle = async (enabled: boolean) => {
    try {
      setShowRecordingNotification(enabled);
      const { Store } = await import('@tauri-apps/plugin-store');
      const store = await Store.load('preferences.json');
      await store.set('show_recording_notification', enabled);
      await store.save();
      toast.success(t('preferenceSaved'));
    } catch (error) {
      console.error('Failed to save notification preference:', error);
      toast.error(t('preferenceSaveFailed'));
    }
  };

  /**
   * Saves the tab's preferences. The toast says what was changed: the devices
   * name the devices, a switch says it was saved, a slider says nothing. When
   * saving fails the screen goes back to what is actually saved, rather than
   * showing a choice the settings do not hold.
   */
  const savePreferences = async (
    prefs: RecordingPreferences,
    change: 'devices' | 'setting' | 'quiet',
  ) => {
    setSaving(true);
    try {
      await invoke('set_recording_preferences', { preferences: prefs });
      savedRef.current = prefs;
      onSave?.(prefs);

      if (change === 'devices') {
        const micDevice = prefs.preferred_mic_device || t('deviceDefault');
        const systemDevice = prefs.preferred_system_device || t('deviceDefault');
        toast.success(t('devicePreferencesSaved'), {
          description: t('devicesSelectedDescription', { mic: micDevice, system: systemDevice })
        });
      } else if (change === 'setting') {
        toast.success(t('preferenceSaved'));
      }
    } catch (error) {
      setPreferences(savedRef.current);
      toastFailure(
        t(change === 'devices' ? 'devicePreferencesSaveFailed' : 'preferenceSaveFailed'),
        'recording-preferences-save',
        error,
      );
    } finally {
      setSaving(false);
    }
  };

  const lockedSwitch = lockedRecordingSwitch(
    preferences.auto_save,
    preferences.live_transcription !== false,
  );
  const echoMode = shownEchoMode(preferences.echo_mode, isWindows);
  const ownSpeechDetectorAvailable = isWindows && echoMode === 'builtin';

  if (loading) {
    return (
      <div className="animate-pulse">
        <div className="h-4 bg-gray-200 rounded w-1/4 mb-4"></div>
        <div className="h-8 bg-gray-200 rounded mb-4"></div>
      </div>
    );
  }

  return (
    <div className="min-w-0 max-w-full space-y-6">
      <div className="min-w-0">
        <h3 className="mb-4 text-lg font-semibold">{t('recordingTitle')}</h3>
        <p className="mb-6 text-sm text-gray-600">
          {t('recordingDescription')}
        </p>
      </div>

      {/* Auto Save Toggle. A recording keeps its sound, its live text or
          both: whichever is the last one on cannot be turned off. */}
      <div className="flex min-w-0 items-start justify-between gap-3 rounded-lg border p-4 sm:items-center">
        <div className="min-w-0 flex-1">
          <div className="font-medium">{t('autoSaveTitle')}</div>
          <div className="text-sm text-gray-600">
            {t('autoSaveDescription')}
          </div>
          {!preferences.auto_save && (
            <div className="mt-2 text-sm text-amber-500">
              {t('autoSaveOffWarning')}
            </div>
          )}
          {lockedSwitch === 'autoSave' && (
            <div className="mt-2 text-xs text-[var(--af-text-3)]">
              {t('autoSaveLockedReason')}
            </div>
          )}
        </div>
        <Switch
          checked={preferences.auto_save}
          onCheckedChange={handleAutoSaveToggle}
          disabled={saving || lockedSwitch === 'autoSave'}
          aria-label={t('autoSaveTitle')}
          className="shrink-0"
        />
      </div>

      {/* Live text, or only sound now and the text afterwards */}
      <div className="flex min-w-0 items-start justify-between gap-3 rounded-lg border p-4 sm:items-center">
        <div className="min-w-0 flex-1">
          <div className="font-medium">{t('liveTextTitle')}</div>
          <div className="text-sm text-gray-600">
            {t('liveTextDescription')}
          </div>
          {lockedSwitch === 'liveText' && (
            <div className="mt-2 text-xs text-[var(--af-text-3)]">
              {t('liveTextLockedReason')}
            </div>
          )}
        </div>
        <Switch
          checked={preferences.live_transcription !== false}
          onCheckedChange={handleLiveTranscriptionToggle}
          disabled={saving || lockedSwitch === 'liveText'}
          aria-label={t('liveTextTitle')}
          className="shrink-0"
        />
      </div>

      {/* One-to-one: trust the channels instead of clustering voices */}
      <div className="flex min-w-0 items-start justify-between gap-3 rounded-lg border p-4 sm:items-center">
        <div className="min-w-0 flex-1">
          <div className="font-medium">{t('singleRemoteSpeakerTitle')}</div>
          <div className="text-sm text-gray-600">
            {t('singleRemoteSpeakerDescription')}
          </div>
        </div>
        <Switch
          checked={preferences.single_remote_speaker !== false}
          onCheckedChange={handleSingleRemoteSpeakerToggle}
          disabled={saving}
          className="shrink-0"
        />
      </div>

      {/* The speakers' echo: one choice, where three switches used to
          override each other while all reading "on". */}
      <div className="min-w-0 space-y-3 rounded-lg border p-4">
        <div className="min-w-0">
          <div className="font-medium">{t('echoModeTitle')}</div>
          <div className="text-sm text-gray-600">
            {t('echoModeDescription')}
          </div>
        </div>
        <div role="radiogroup" aria-label={t('echoModeTitle')} className="space-y-2">
          <EchoModeOption
            selected={echoMode === 'windows'}
            disabled={saving}
            onSelect={() => void handleEchoModeChange('windows')}
            title={t('echoModeWindows')}
            hint={t('echoModeWindowsHint')}
            unavailable={isWindows ? undefined : t('echoModeWindowsOnly')}
          />
          <EchoModeOption
            selected={echoMode === 'builtin'}
            disabled={saving}
            onSelect={() => void handleEchoModeChange('builtin')}
            title={t('echoModeBuiltin')}
            hint={t('echoModeBuiltinHint')}
          />
          <EchoModeOption
            selected={echoMode === 'off'}
            disabled={saving}
            onSelect={() => void handleEchoModeChange('off')}
            title={t('echoModeOff')}
            hint={t('echoModeOffHint')}
          />
        </div>

        {/* The own-speech detector: an addition to the built-in canceller,
            and only that. With Windows cleaning the microphone it would
            have to turn that cleaning off, which is what made the old
            switches contradict each other. */}
        <div className="flex min-w-0 items-start justify-between gap-3 border-t pt-3">
          <div className="min-w-0 flex-1">
            <div className="text-sm font-medium">{t('ownSpeechDetectorTitle')}</div>
            <div className="text-xs text-gray-600">
              {ownSpeechDetectorAvailable
                ? t('ownSpeechDetectorDescription')
                : t('ownSpeechDetectorUnavailable')}
            </div>
          </div>
          <Switch
            checked={ownSpeechDetectorAvailable && preferences.own_speech_detector === true}
            onCheckedChange={handleOwnSpeechDetectorToggle}
            disabled={saving || !ownSpeechDetectorAvailable}
            aria-label={t('ownSpeechDetectorTitle')}
            className="shrink-0"
          />
        </div>
      </div>

      {/* Text-level echo fallback — only for setups the canceller cannot serve */}
      <div className="flex min-w-0 items-start justify-between gap-3 rounded-lg border p-4 sm:items-center">
        <div className="min-w-0 flex-1">
          <div className="font-medium">{t('echoTextFilterTitle')}</div>
          <div className="text-sm text-gray-600">
            {t('echoTextFilterDescription')}
          </div>
        </div>
        <Switch
          checked={preferences.echo_text_filter === true}
          onCheckedChange={handleEchoTextFilterToggle}
          disabled={saving}
          className="shrink-0"
        />
      </div>

      {/* Mic gain — boost local voice after loudness normalize */}
      <div className="min-w-0 space-y-3 rounded-lg border p-4">
        <div className="flex min-w-0 items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="font-medium">{t('micGainTitle')}</div>
            <div className="text-sm text-gray-600 break-words">
              {t('micGainDescription')}
            </div>
          </div>
          <span className="shrink-0 text-sm font-semibold tabular-nums text-[var(--af-text)]">
            {(preferences.mic_gain ?? 1).toFixed(1)}×
          </span>
        </div>
        <input
          type="range"
          min={0.5}
          max={3}
          step={0.1}
          value={preferences.mic_gain ?? 1}
          disabled={saving}
          onChange={(e) => {
            const v = parseFloat(e.target.value);
            setPreferences((p) => ({ ...p, mic_gain: v }));
          }}
          onMouseUp={(e) => void handleMicGainChange(parseFloat((e.target as HTMLInputElement).value))}
          onTouchEnd={(e) => void handleMicGainChange(parseFloat((e.target as HTMLInputElement).value))}
          onBlur={(e) => void handleMicGainChange(parseFloat(e.target.value))}
          className="w-full min-w-0 max-w-full accent-[var(--af-accent,#4a8bff)]"
        />
        <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-gray-500">
          <span>{t('gainQuieter')}</span>
          <button
            type="button"
            className="underline hover:text-gray-800"
            disabled={saving}
            onClick={() => void handleMicGainChange(1)}
          >
            {t('gainReset')}
          </button>
          <span>{t('gainLouder')}</span>
        </div>
      </div>

      {/* System gain — applied before meters, transcription, and saved tracks */}
      <div className="min-w-0 space-y-3 rounded-lg border p-4">
        <div className="flex min-w-0 items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="font-medium">{t('systemGainTitle')}</div>
            <div className="text-sm text-gray-600 break-words">
              {t('systemGainDescription')}
            </div>
          </div>
          <span className="shrink-0 text-sm font-semibold tabular-nums text-[var(--af-text)]">
            {(preferences.system_gain ?? 1).toFixed(1)}×
          </span>
        </div>
        <input
          type="range"
          min={0.5}
          max={3}
          step={0.1}
          value={preferences.system_gain ?? 1}
          disabled={saving}
          onChange={(e) => {
            const v = parseFloat(e.target.value);
            setPreferences((p) => ({ ...p, system_gain: v }));
          }}
          onMouseUp={(e) => void handleSystemGainChange(parseFloat((e.target as HTMLInputElement).value))}
          onTouchEnd={(e) => void handleSystemGainChange(parseFloat((e.target as HTMLInputElement).value))}
          onBlur={(e) => void handleSystemGainChange(parseFloat(e.target.value))}
          className="w-full min-w-0 max-w-full accent-[var(--af-accent,#4a8bff)]"
        />
        <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-gray-500">
          <span>{t('gainQuieter')}</span>
          <button
            type="button"
            className="underline hover:text-gray-800"
            disabled={saving}
            onClick={() => void handleSystemGainChange(1)}
          >
            {t('gainReset')}
          </button>
          <span>{t('gainLouder')}</span>
        </div>
        <p className="text-xs text-amber-700">
          {t('systemGainLimiterNote')}
        </p>
      </div>

      {/* Folder Location - Only shown when auto_save is enabled */}
      {preferences.auto_save && (
        <div className="min-w-0 space-y-4">
          <div className="min-w-0 rounded-lg border bg-gray-50 p-4">
            <div className="mb-2 font-medium">{t('saveLocation')}</div>
            <div className="mb-3 break-all text-sm text-gray-600">
              {preferences.save_folder || t('defaultFolder')}
            </div>
            <div className="flex flex-wrap gap-2">
              <button
                onClick={handleChangeFolder}
                disabled={isChoosingFolder || saving}
                className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-50 transition-colors disabled:cursor-not-allowed disabled:opacity-60"
              >
                <FolderCog className="w-4 h-4" />
                {isChoosingFolder ? t('storageChoosing') : t('storageChangeFolder')}
              </button>
              <button
                onClick={handleOpenFolder}
                disabled={isChoosingFolder}
                className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-50 transition-colors disabled:cursor-not-allowed disabled:opacity-60"
              >
                <FolderOpen className="w-4 h-4" />
                {t('storageOpenFolder')}
              </button>
            </div>
          </div>

          <div className="p-4 border rounded-lg bg-blue-50">
            <div className="text-sm text-blue-800">
              <strong>{t('fileFormat')}</strong> {t('fileFormatValue', { format: preferences.file_format.toUpperCase() })}
            </div>
            <div className="text-xs text-blue-600 mt-1">
              {t('fileNamePattern', { format: preferences.file_format })}
            </div>
          </div>
        </div>
      )}

      {/* Device Preferences */}
      <div className="space-y-4">
        <div className="border-t pt-6">
          <h4 className="text-base font-medium text-gray-900 mb-4">{t('defaultAudioDevices')}</h4>
          <p className="text-sm text-gray-600 mb-4">
            {t('defaultAudioDevicesDescription')}
          </p>

          <div className="border rounded-lg p-4 bg-gray-50">
            <DeviceSelection
              selectedDevices={{
                micDevice: preferences.preferred_mic_device,
                systemDevice: preferences.preferred_system_device
              }}
              onDeviceChange={handleDeviceChange}
              disabled={saving}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

import React, { useState, useEffect } from 'react';
import { useTranslations } from 'next-intl';
import { Switch } from '@/components/ui/switch';
import { FolderCog, FolderOpen } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { DeviceSelection, SelectedDevices } from '@/components/DeviceSelection';
import Analytics from '@/lib/analytics';
import { toast } from 'sonner';
import { useConfig } from '@/contexts/ConfigContext';

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
  /** Cancel the speakers' echo out of the microphone channel. */
  echo_cancellation?: boolean;
  /** Drop microphone text that only repeats a recent system phrase. */
  echo_text_filter?: boolean;
  /** One conversation partner: microphone is you, the speakers are them. */
  single_remote_speaker?: boolean;
  /** Let Windows remove the speakers' echo before we receive the sound. */
  system_echo_cancellation?: boolean;
}

interface RecordingSettingsProps {
  onSave?: (preferences: RecordingPreferences) => void;
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
    echo_cancellation: true,
    echo_text_filter: false,
    single_remote_speaker: true,
    system_echo_cancellation: true,
  });
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
    await savePreferences(newPreferences);

    // Track auto-save setting change
    await Analytics.track('auto_save_recording_toggled', {
      enabled: enabled.toString()
    });
  };

  const handleSystemEchoCancellationToggle = async (enabled: boolean) => {
    const newPreferences = { ...preferences, system_echo_cancellation: enabled };
    setPreferences(newPreferences);
    await savePreferences(newPreferences);
  };

  const handleEchoCancellationToggle = async (enabled: boolean) => {
    const newPreferences = { ...preferences, echo_cancellation: enabled };
    setPreferences(newPreferences);
    await savePreferences(newPreferences);
  };

  const handleSingleRemoteSpeakerToggle = async (enabled: boolean) => {
    const newPreferences = { ...preferences, single_remote_speaker: enabled };
    setPreferences(newPreferences);
    await savePreferences(newPreferences);
  };

  const handleEchoTextFilterToggle = async (enabled: boolean) => {
    const newPreferences = { ...preferences, echo_text_filter: enabled };
    setPreferences(newPreferences);
    await savePreferences(newPreferences);
  };

  const handleMicGainChange = async (value: number) => {
    const mic_gain = Math.min(3, Math.max(0.5, value));
    const newPreferences = { ...preferences, mic_gain };
    setPreferences(newPreferences);
    await savePreferences(newPreferences);
  };

  const handleSystemGainChange = async (value: number) => {
    const system_gain = Math.min(3, Math.max(0.5, value));
    const newPreferences = { ...preferences, system_gain };
    setPreferences(newPreferences);
    await savePreferences(newPreferences);
  };

  const handleDeviceChange = async (devices: SelectedDevices) => {
    const newPreferences = {
      ...preferences,
      preferred_mic_device: devices.micDevice,
      preferred_system_device: devices.systemDevice
    };
    setPreferences(newPreferences);
    await savePreferences(newPreferences);

    // Track default device preference changes
    // Note: Individual device selection analytics are tracked in DeviceSelection component
    await Analytics.track('default_devices_changed', {
      has_preferred_microphone: (!!devices.micDevice).toString(),
      has_preferred_system_audio: (!!devices.systemDevice).toString()
    });
  };

  const handleOpenFolder = async () => {
    try {
      await invoke('open_recordings_folder');
    } catch (error) {
      console.error('Failed to open recordings folder:', error);
      toast.error(t('openRecordingsFolderFailed'), {
        description: String(error),
      });
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
      setPreferences(newPreferences);
      updateRecordingsLocation(selectedFolder);
      onSave?.(newPreferences);
      toast.success(t('recordingsFolderUpdated'));
      Analytics.track('recordings_folder_changed', { source: 'recording_settings' }).catch(console.error);
    } catch (error) {
      console.error('Failed to change recordings folder:', error);
      toast.error(t('recordingsFolderUpdateFailed'), {
        description: String(error),
      });
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
      await Analytics.track('recording_notification_preference_changed', {
        enabled: enabled.toString()
      });
    } catch (error) {
      console.error('Failed to save notification preference:', error);
      toast.error(t('preferenceSaveFailed'));
    }
  };

  const savePreferences = async (prefs: RecordingPreferences) => {
    setSaving(true);
    try {
      await invoke('set_recording_preferences', { preferences: prefs });
      onSave?.(prefs);

      // Show success toast with device details
      const micDevice = prefs.preferred_mic_device || t('deviceDefault');
      const systemDevice = prefs.preferred_system_device || t('deviceDefault');
      toast.success(t('devicePreferencesSaved'), {
        description: t('devicesSelectedDescription', { mic: micDevice, system: systemDevice })
      });
    } catch (error) {
      console.error('Failed to save recording preferences:', error);
      toast.error(t('devicePreferencesSaveFailed'), {
        description: error instanceof Error ? error.message : String(error)
      });
    } finally {
      setSaving(false);
    }
  };

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

      {/* Auto Save Toggle */}
      <div className="flex min-w-0 items-start justify-between gap-3 rounded-lg border p-4 sm:items-center">
        <div className="min-w-0 flex-1">
          <div className="font-medium">{t('autoSaveTitle')}</div>
          <div className="text-sm text-gray-600">
            {t('autoSaveDescription')}
          </div>
        </div>
        <Switch
          checked={preferences.auto_save}
          onCheckedChange={handleAutoSaveToggle}
          disabled={saving}
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

      {/* Windows' own echo cancellation. Above ours on purpose: when this
          is on, ours stands down, because it has access to the played
          signal and the exact delay and we never will. */}
      <div className="flex min-w-0 items-start justify-between gap-3 rounded-lg border p-4 sm:items-center">
        <div className="min-w-0 flex-1">
          <div className="font-medium">{t('systemEchoCancellationTitle')}</div>
          <div className="text-sm text-gray-600">
            {t('systemEchoCancellationDescription')}
          </div>
        </div>
        <Switch
          checked={preferences.system_echo_cancellation !== false}
          onCheckedChange={handleSystemEchoCancellationToggle}
          disabled={saving}
          className="shrink-0"
        />
      </div>

      {/* Echo cancellation — keep the speakers out of the mic channel */}
      <div className="flex min-w-0 items-start justify-between gap-3 rounded-lg border p-4 sm:items-center">
        <div className="min-w-0 flex-1">
          <div className="font-medium">{t('echoCancellationTitle')}</div>
          <div className="text-sm text-gray-600">
            {t('echoCancellationDescription')}
          </div>
        </div>
        <Switch
          checked={preferences.echo_cancellation !== false}
          onCheckedChange={handleEchoCancellationToggle}
          disabled={saving}
          className="shrink-0"
        />
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

      {/* Info when auto_save is disabled */}
      {!preferences.auto_save && (
        <div className="p-4 border rounded-lg bg-yellow-50">
          <div className="text-sm text-yellow-800">
            {t('autoSaveDisabledNote')}
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

"use client"

import { useEffect, useState } from "react"
import { Switch } from "./ui/switch"
import { Button } from "./ui/button"
import { ButtonGroup } from "./ui/button-group"
import { FolderCog, FolderOpen } from "lucide-react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { useConfig, NotificationSettings } from "@/contexts/ConfigContext"
import { applyAppTheme, getSavedAppTheme, type AppTheme } from "@/lib/app-theme"
import { useAppLocale } from "@/contexts/LocaleContext"
import { useTranslations } from "next-intl"

export function PreferenceSettings() {
  const t = useTranslations('settings');
  const { locale, setLocale } = useAppLocale();
  const {
    notificationSettings,
    storageLocations,
    isLoadingPreferences,
    loadPreferences,
    updateNotificationSettings,
    updateRecordingsLocation,
  } = useConfig();
  const [isChoosingRecordingsFolder, setIsChoosingRecordingsFolder] = useState(false);

  const [notificationsEnabled, setNotificationsEnabled] = useState<boolean | null>(null);
  const [isInitialLoad, setIsInitialLoad] = useState(true);
  const [previousNotificationsEnabled, setPreviousNotificationsEnabled] = useState<boolean | null>(null);

  // "Your name" — used to label the user's mic transcripts as "You (Name)".
  const [userName, setUserName] = useState<string>('');
  useEffect(() => {
    if (typeof window !== 'undefined') {
      setUserName(localStorage.getItem('meetily_user_name') || '');
    }
  }, []);
  const saveUserName = (v: string) => {
    setUserName(v);
    if (typeof window !== 'undefined') {
      localStorage.setItem('meetily_user_name', v);
    }
  };

  // Theme (default dark): light, dark navy, or AMOLED true black.
  const [theme, setTheme] = useState<AppTheme>('dark');
  useEffect(() => {
    setTheme(getSavedAppTheme());
  }, []);
  const selectTheme = (next: AppTheme) => {
    setTheme(next);
    applyAppTheme(next, true);
  };

  // Lazy load preferences on mount (only loads if not already cached)
  useEffect(() => {
    loadPreferences();
  }, [loadPreferences]);

  // Update notificationsEnabled when notificationSettings are loaded from global state
  useEffect(() => {
    if (notificationSettings) {
      // Notification enabled means both started and stopped notifications are enabled
      const enabled =
        notificationSettings.notification_preferences.show_recording_started &&
        notificationSettings.notification_preferences.show_recording_stopped;
      setNotificationsEnabled(enabled);
      if (isInitialLoad) {
        setPreviousNotificationsEnabled(enabled);
        setIsInitialLoad(false);
      }
    } else if (!isLoadingPreferences) {
      // If not loading and no settings, use default
      setNotificationsEnabled(true);
      if (isInitialLoad) {
        setPreviousNotificationsEnabled(true);
        setIsInitialLoad(false);
      }
    }
  }, [notificationSettings, isLoadingPreferences, isInitialLoad])

  useEffect(() => {
    // Skip update on initial load or if value hasn't actually changed
    if (isInitialLoad || notificationsEnabled === null || notificationsEnabled === previousNotificationsEnabled) return;
    if (!notificationSettings) return;

    const handleUpdateNotificationSettings = async () => {
      console.log("Updating notification settings to:", notificationsEnabled);

      try {
        // Update the notification preferences
        const updatedSettings: NotificationSettings = {
          ...notificationSettings,
          notification_preferences: {
            ...notificationSettings.notification_preferences,
            show_recording_started: notificationsEnabled,
            show_recording_stopped: notificationsEnabled,
          }
        };

        console.log("Calling updateNotificationSettings with:", updatedSettings);
        await updateNotificationSettings(updatedSettings);
        setPreviousNotificationsEnabled(notificationsEnabled);
        console.log("Successfully updated notification settings to:", notificationsEnabled);
      } catch (error) {
        console.error('Failed to update notification settings:', error);
      }
    };

    handleUpdateNotificationSettings();
  }, [notificationsEnabled, notificationSettings, isInitialLoad, previousNotificationsEnabled, updateNotificationSettings])

  const handleOpenFolder = async (folderType: 'database' | 'models' | 'recordings') => {
    try {
      switch (folderType) {
        case 'database':
          await invoke('open_database_folder');
          break;
        case 'models':
          await invoke('open_models_folder');
          break;
        case 'recordings':
          await invoke('open_recordings_folder');
          break;
      }
    } catch (error) {
      console.error(`Failed to open ${folderType} folder:`, error);
    }
  };

  const handleChangeRecordingsFolder = async () => {
    if (isChoosingRecordingsFolder) return;

    setIsChoosingRecordingsFolder(true);
    try {
      const selectedFolder = await invoke<string | null>('select_recording_folder');
      if (!selectedFolder) return;

      const preferences = await invoke<Record<string, unknown> & { save_folder: string }>(
        'get_recording_preferences',
      );
      await invoke('set_recording_preferences', {
        preferences: { ...preferences, save_folder: selectedFolder },
      });
      updateRecordingsLocation(selectedFolder);
      toast.success(t('recordingsFolderUpdated'));
    } catch (error) {
      console.error('Failed to change recordings folder:', error);
      toast.error(t('recordingsFolderUpdateFailed'), {
        description: String(error),
      });
    } finally {
      setIsChoosingRecordingsFolder(false);
    }
  };

  // Show loading only if we're actually loading and don't have cached data
  if (isLoadingPreferences && !notificationSettings && !storageLocations) {
    return <div className="max-w-2xl mx-auto p-6">{t('loadingPreferences')}</div>
  }

  // Show loading if notificationsEnabled hasn't been determined yet
  if (notificationsEnabled === null && !isLoadingPreferences) {
    return <div className="max-w-2xl mx-auto p-6">{t('loadingPreferences')}</div>
  }

  // Ensure we have a boolean value for the Switch component
  const notificationsEnabledValue = notificationsEnabled ?? false;

  return (
    <div className="space-y-6">
      {/* Interface language. Switching is instant — the locale lives in React
          context, so no reload is needed. */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <div className="flex items-center justify-between gap-4 flex-wrap">
          <div>
            <h3 className="text-lg font-semibold text-gray-900 mb-2">{t('language')}</h3>
            <p className="text-sm text-gray-600">{t('languageDescription')}</p>
          </div>
          <ButtonGroup>
            <Button
              type="button"
              size="sm"
              variant={locale === 'ru' ? 'default' : 'outline'}
              onClick={() => setLocale('ru')}
            >
              {t('languageRussian')}
            </Button>
            <Button
              type="button"
              size="sm"
              variant={locale === 'en' ? 'default' : 'outline'}
              onClick={() => setLocale('en')}
            >
              {t('languageEnglish')}
            </Button>
          </ButtonGroup>
        </div>
      </div>

      {/* Appearance / Theme Section */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <div className="flex items-center justify-between gap-4 flex-wrap">
          <div>
            <h3 className="text-lg font-semibold text-gray-900 mb-2">{t('theme')}</h3>
            <p className="text-sm text-gray-600">{t('themeDescription')}</p>
          </div>
          <ButtonGroup>
            <Button
              type="button"
              size="sm"
              variant={theme === 'light' ? 'default' : 'outline'}
              onClick={() => selectTheme('light')}
            >
              {t('themeLight')}
            </Button>
            <Button
              type="button"
              size="sm"
              variant={theme === 'dark' ? 'default' : 'outline'}
              onClick={() => selectTheme('dark')}
            >
              {t('themeDark')}
            </Button>
            <Button
              type="button"
              size="sm"
              variant={theme === 'amoled' ? 'default' : 'outline'}
              onClick={() => selectTheme('amoled')}
            >
              {t('themeAmoled')}
            </Button>
          </ButtonGroup>
        </div>
      </div>

      {/* Your Name Section */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <h3 className="text-lg font-semibold text-gray-900 mb-2">{t('yourName')}</h3>
        <p className="text-sm text-gray-600 mb-3">
          {t.rich('yourNameDescription', {
            b: () => (
              <strong>{t('yourNameValue', { name: userName || t('yourNameFallback') })}</strong>
            ),
          })}
        </p>
        <input
          type="text"
          value={userName}
          onChange={(e) => saveUserName(e.target.value)}
          placeholder={t('yourNamePlaceholder')}
          className="w-full max-w-sm rounded-lg border border-gray-200 px-3 py-2 text-sm focus:border-blue-400 focus:outline-none"
        />
      </div>

      {/* Notifications Section */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-lg font-semibold text-gray-900 mb-2">{t('notifications')}</h3>
            <p className="text-sm text-gray-600">{t('notificationsDescription')}</p>
          </div>
          <Switch checked={notificationsEnabledValue} onCheckedChange={setNotificationsEnabled} />
        </div>
      </div>

      {/* Data Storage Locations Section */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <h3 className="text-lg font-semibold text-gray-900 mb-4">{t('storageTitle')}</h3>
        <p className="text-sm text-gray-600 mb-6">
          {t('storageDescription')}
        </p>

        <div className="space-y-4">
          {/* Database Location */}
          {/* <div className="p-4 border rounded-lg bg-gray-50">
            <div className="font-medium mb-2">Database</div>
            <div className="text-sm text-gray-600 mb-3 break-all font-mono text-xs">
              {storageLocations?.database || 'Loading...'}
            </div>
            <button
              onClick={() => handleOpenFolder('database')}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              Open Folder
            </button>
          </div> */}

          {/* Models Location */}
          {/* <div className="p-4 border rounded-lg bg-gray-50">
            <div className="font-medium mb-2">Whisper Models</div>
            <div className="text-sm text-gray-600 mb-3 break-all font-mono text-xs">
              {storageLocations?.models || 'Loading...'}
            </div>
            <button
              onClick={() => handleOpenFolder('models')}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              Open Folder
            </button>
          </div> */}

          {/* Recordings Location */}
          <div className="p-4 border rounded-lg bg-gray-50">
            <div className="font-medium mb-2">{t('storageRecordings')}</div>
            <div className="text-sm text-gray-600 mb-3 break-all font-mono text-xs">
              {storageLocations?.recordings || t('storageLoading')}
            </div>
            <div className="flex flex-wrap gap-2">
              <button
                onClick={handleChangeRecordingsFolder}
                disabled={isChoosingRecordingsFolder}
                className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors disabled:cursor-not-allowed disabled:opacity-60"
              >
                <FolderCog className="w-4 h-4" />
                {isChoosingRecordingsFolder ? t('storageChoosing') : t('storageChangeFolder')}
              </button>
              <button
                onClick={() => handleOpenFolder('recordings')}
                className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors"
              >
                <FolderOpen className="w-4 h-4" />
                {t('storageOpenFolder')}
              </button>
            </div>
          </div>
        </div>

        <div className="mt-4 p-3 bg-blue-50 rounded-md">
          <p className="text-xs text-blue-800">
            {t.rich('storagePortableNote', { b: (chunks) => <strong>{chunks}</strong> })}
          </p>
        </div>
      </div>
    </div>
  )
}

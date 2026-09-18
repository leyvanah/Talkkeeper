'use client'

import React, { createContext, useContext, useState, useCallback, useEffect } from 'react';
import { useTranslations } from 'next-intl';
import { useUpdateCheck } from '@/hooks/useUpdateCheck';
import { UpdateInfo } from '@/services/updateService';
import { UPDATES_AVAILABLE } from '@/lib/updates';
import { UpdateDialog } from './UpdateDialog';
import { setUpdateDialogCallback, showUpdateNotification } from './UpdateNotification';
import { invoke } from '@tauri-apps/api/core';
import { usePlatform } from '@/hooks/usePlatform';
import { toast } from 'sonner';
import type { CudaReconfigurationStatus } from '@/lib/transcription-acceleration';

interface UpdateCheckContextType {
  updateInfo: UpdateInfo | null;
  isChecking: boolean;
  checkForUpdates: (force?: boolean) => Promise<void>;
  showUpdateDialog: () => void;
}

const UpdateCheckContext = createContext<UpdateCheckContextType | undefined>(undefined);

export function UpdateCheckProvider({
  children,
  onboardingCompleted = false,
}: {
  children: React.ReactNode;
  onboardingCompleted?: boolean;
}) {
  const t = useTranslations('app');
  const platform = usePlatform();
  // macOS ships as a separate DMG release with no Tauri updater artifact or
  // latest.json entry. Calling the Windows updater path there is misleading.
  const updatesSupported = UPDATES_AVAILABLE && platform !== 'macos';
  const [showDialog, setShowDialog] = useState(false);
  const [checkOnMount, setCheckOnMount] = useState(false);

  const handleShowDialog = useCallback(() => {
    setShowDialog(true);
  }, []);

  useEffect(() => {
    if (!updatesSupported) {
      setCheckOnMount(false);
      return;
    }
    invoke<boolean>('get_check_updates_on_launch')
      .then(setCheckOnMount)
      .catch(() => setCheckOnMount(false));
  }, [updatesSupported]);

  useEffect(() => {
    if (platform !== 'windows' || !onboardingCompleted) return;

    let cancelled = false;
    invoke<CudaReconfigurationStatus>('get_cuda_reconfiguration_status')
      .then((status) => {
        if (cancelled) return;

        const reconfigurationUrl = status.setupDownloadUrl;
        if (status.reconfigurationRequired && reconfigurationUrl) {
          toast.warning(t('cudaReadyTitle'), {
            id: 'cuda-reconfiguration-status',
            description: t('cudaReadyDescription', { backend: status.compiledBackend }),
            duration: 30000,
            action: {
              label: t('cudaDownloadSetup'),
              onClick: () => {
                void invoke('open_external_url', { url: reconfigurationUrl });
              },
            },
          });
        } else if (status.driverUpdateRequired) {
          toast.warning(t('cudaDriverUpdateTitle'), {
            id: 'cuda-reconfiguration-status',
            description: t('cudaDriverUpdateDescription'),
            duration: 30000,
            action: {
              label: t('cudaGetDriver'),
              onClick: () => {
                void invoke('open_external_url', {
                  url: 'https://www.nvidia.com/Download/index.aspx',
                });
              },
            },
          });
        }
      })
      .catch((error) => console.error('Failed to recheck CUDA availability:', error));

    return () => {
      cancelled = true;
    };
  }, [onboardingCompleted, platform]);

  const { updateInfo, isChecking, checkForUpdates } = useUpdateCheck({
    checkOnMount: updatesSupported && checkOnMount,
    showNotification: false,
    onUpdateAvailable: (info) => {
      showUpdateNotification(info, handleShowDialog);
    },
  });

  const checkForSupportedUpdates = useCallback(
    async (force = false) => {
      if (!updatesSupported) return;
      await checkForUpdates(force);
    },
    [checkForUpdates, updatesSupported],
  );

  useEffect(() => {
    // Register the callback so UpdateNotification can trigger the dialog
    setUpdateDialogCallback(handleShowDialog);
    return () => {
      setUpdateDialogCallback(() => {});
    };
  }, [handleShowDialog]);

  // Listen for tray menu events
  useEffect(() => {
    const handleTrayCheck = () => {
      if (!updatesSupported) return;
      void checkForSupportedUpdates(true);
      setShowDialog(true);
    };

    window.addEventListener('check-updates-from-tray', handleTrayCheck);
    return () => window.removeEventListener('check-updates-from-tray', handleTrayCheck);
  }, [checkForSupportedUpdates, updatesSupported]);

  return (
    <UpdateCheckContext.Provider
      value={{
        updateInfo,
        isChecking,
        checkForUpdates: checkForSupportedUpdates,
        showUpdateDialog: handleShowDialog,
      }}
    >
      {children}
      <UpdateDialog
        open={showDialog}
        onOpenChange={setShowDialog}
        updateInfo={updateInfo}
      />
    </UpdateCheckContext.Provider>
  );
}

export function useUpdateCheckContext() {
  const context = useContext(UpdateCheckContext);
  if (context === undefined) {
    throw new Error('useUpdateCheckContext must be used within UpdateCheckProvider');
  }
  return context;
}

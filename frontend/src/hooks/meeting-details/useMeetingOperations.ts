import { useCallback } from 'react';
import { invoke as invokeTauri } from '@tauri-apps/api/core';
import { useTranslations } from 'next-intl';
import { toastFailure } from '@/lib/failure';

interface UseMeetingOperationsProps {
  meeting: any;
}

export function useMeetingOperations({
  meeting,
}: UseMeetingOperationsProps) {
  const t = useTranslations('meetingDetails');

  // Open meeting folder in file explorer
  const handleOpenMeetingFolder = useCallback(async () => {
    try {
      await invokeTauri('open_meeting_folder', { meetingId: meeting.id });
    } catch (error) {
      toastFailure(t('openRecordingFolderFailed'), 'meeting-folder-open', error);
    }
  }, [meeting.id, t]);

  return {
    handleOpenMeetingFolder,
  };
}

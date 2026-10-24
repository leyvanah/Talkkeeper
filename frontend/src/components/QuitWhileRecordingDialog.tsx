'use client';

/**
 * Asked when Quit is chosen in the tray while a recording is on or paused
 * (see quit.rs). Stopping goes through the backend: it stops the recording
 * as the tray's Stop does, waits for this window to save the meeting, then
 * exits. Cancel leaves the recording running.
 */

import { useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';

export function QuitWhileRecordingDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const t = useTranslations('app');

  const stopAndQuit = async () => {
    onClose();
    toast.message(t('quitDuringRecordingSaving'), { id: 'quit-during-recording', duration: 60000 });
    try {
      await invoke('stop_recording_and_quit');
    } catch (error) {
      console.error('Stop and quit failed:', error);
      toast.error(t('quitDuringRecordingFailed'), {
        id: 'quit-during-recording',
        description: error instanceof Error ? error.message : String(error),
        duration: 10000,
      });
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent aria-describedby="quit-during-recording-description" className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t('quitDuringRecordingTitle')}</DialogTitle>
          <DialogDescription id="quit-during-recording-description">
            {t('quitDuringRecordingDescription')}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            {t('quitDuringRecordingCancel')}
          </Button>
          <Button onClick={() => void stopAndQuit()}>
            {t('quitDuringRecordingStop')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

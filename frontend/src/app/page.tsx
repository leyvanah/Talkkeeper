'use client';

import { useState, useEffect } from 'react';
import { useTranslations } from 'next-intl';
import { useWindowTitle } from '@/components/AppHeader';
import { motion } from 'framer-motion';
import { RecordingControls } from '@/components/RecordingControls';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState, RecordingStatus } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useConfig } from '@/contexts/ConfigContext';
import { StatusOverlays } from '@/app/_components/StatusOverlays';
import { SettingsModals } from './_components/SettingsModal';
import { TranscriptPanel } from './_components/TranscriptPanel';
import { useModalState } from '@/hooks/useModalState';
import { useRecordingStateSync } from '@/hooks/useRecordingStateSync';
import { useRecordingStart } from '@/hooks/useRecordingStart';
import { useRecordingStop } from '@/hooks/useRecordingStop';
import { useTranscriptRecovery } from '@/hooks/useTranscriptRecovery';
import { TranscriptRecovery } from '@/components/TranscriptRecovery';
import { usePostCall } from '@/contexts/PostCallContext';
import { deleteLegacyRecoveryStore } from '@/lib/unsaved-recordings';
import { toast } from 'sonner';
import { useRouter } from 'next/navigation';
import { NotesEditor } from '@/components/Notes/NotesEditor';
import { useRecordingNotes } from '@/hooks/useMeetingNotes';
import { toastFailure } from '@/lib/failure';

/** Whether the notes beside a recording are open; a habit, kept per machine. */
const NOTES_OPEN_KEY = 'recording_notes_open';
/** The notes column: narrower in a small window, so the transcript keeps room. */
const NOTES_WIDTH = 'clamp(15rem, 26vw, 20rem)';

export default function Home() {
  const t = useTranslations('home');
  const tn = useTranslations('notes');
  // Local page state (not moved to contexts)
  const [isRecording, setIsRecordingState] = useState(false);
  const [showRecoveryDialog, setShowRecoveryDialog] = useState(false);

  // Use contexts for state management
  const { meetingTitle } = useTranscripts();

  // What is on screen, for the window header and the taskbar.
  // Before a recording has a name, the page is simply where a new one starts.
  const ts = useTranslations('sidebar');
  useWindowTitle(meetingTitle || ts('newRecording'));
  const { transcriptModelConfig, selectedDevices } = useConfig();
  const recordingState = useRecordingState();

  // Extract status from global state
  const { status, isStopping, isProcessing, isSaving } = recordingState;

  // The owner's notes, written beside the recording and kept with it.
  const recordingNotes = useRecordingNotes(recordingState.isRecording);
  const [notesOpen, setNotesOpen] = useState(true);
  useEffect(() => {
    try {
      if (localStorage.getItem(NOTES_OPEN_KEY) === 'false') setNotesOpen(false);
    } catch {
      // Without storage the notes simply start open.
    }
  }, []);
  const toggleNotes = () => {
    setNotesOpen((open) => {
      try {
        localStorage.setItem(NOTES_OPEN_KEY, String(!open));
      } catch {
        // Remembering is a convenience.
      }
      return !open;
    });
  };
  const showNotes = recordingState.isRecording && notesOpen;

  // Hooks
  const { setIsMeetingActive, refetchMeetings } = useSidebar();
  const { begin: beginPostCall } = usePostCall();
  const { modals, messages, showModal, hideModal } = useModalState(transcriptModelConfig);
  const { isRecordingDisabled, setIsRecordingDisabled } = useRecordingStateSync(isRecording, setIsRecordingState, setIsMeetingActive);
  const { handleRecordingStart } = useRecordingStart(isRecording, setIsRecordingState, showModal);

  // Get handleRecordingStop function and setIsStopping (state comes from global context)
  const { handleRecordingStop, setIsStopping } = useRecordingStop(
    setIsRecordingState,
    setIsRecordingDisabled
  );

  // Recovery hook
  const {
    recoverableMeetings,
    isLoading: isLoadingRecovery,
    isRecovering,
    checkForRecoverableTranscripts,
    recoverMeeting,
    loadMeetingTranscripts,
    deleteRecoverableMeeting,
    openWithOtherKey,
  } = useTranscriptRecovery();

  const router = useRouter();

  useEffect(() => {
  }, []);

  // Startup recovery check
  useEffect(() => {
    const performStartupChecks = async () => {
      try {
        // Skip recovery check if currently recording or processing stop
        // This prevents the recovery dialog from showing when:
        if (recordingState.isRecording ||
          status === RecordingStatus.STOPPING ||
          status === RecordingStatus.PROCESSING_TRANSCRIPTS ||
          status === RecordingStatus.SAVING) {
          console.log('Skipping recovery check - recording in progress or processing');
          return;
        }

        // 1. The window used to keep every line of a recording in IndexedDB,
        //    in the clear. Recovery reads the backend journal now; the old
        //    store is deleted wherever it is still found.
        await deleteLegacyRecoveryStore();
        sessionStorage.removeItem('indexeddb_current_meeting_id');

        // 2. Always check for recoverable meetings on startup
        // Don't skip based on sessionStorage - we need to check every time
        await checkForRecoverableTranscripts();
      } catch (error) {
        console.error('Failed to perform startup checks:', error);
      }
    };

    performStartupChecks();
  }, [checkForRecoverableTranscripts, recordingState.isRecording, status]);

  // Watch for recoverable meetings changes and show dialog once per session
  useEffect(() => {
    // Only show dialog if we have meetings and haven't shown it yet this session
    if (recoverableMeetings.length > 0) {
      const shownThisSession = sessionStorage.getItem('recovery_dialog_shown');
      if (!shownThisSession) {
        setShowRecoveryDialog(true);
        sessionStorage.setItem('recovery_dialog_shown', 'true');
      }
    }
  }, [recoverableMeetings]);

  // Handle recovery with toast notifications and navigation
  const handleRecovery = async (meetingId: string) => {
    // Others still waiting: stay with the list rather than leave for the
    // recovered meeting, which the toast offers to open instead.
    const othersLeft = recoverableMeetings.some((meeting) => meeting.meetingId !== meetingId);
    try {
      const result = await recoverMeeting(meetingId);

      if (result.success && result.meetingId && result.transcribeFolder) {
        // Only the audio was left: recognise it the way a stopped recording is.
        beginPostCall(result.meetingId, result.transcribeFolder);
        const recoveredId = result.meetingId;
        toast.success(t('restoredFromAudioTitle'), {
          description: t('restoredFromAudioDescription'),
          action: othersLeft ? {
            label: t('viewMeetingAction'),
            onClick: () => router.push(`/meeting-details?id=${encodeURIComponent(recoveredId)}`),
          } : undefined,
        });
        if (!othersLeft) sessionStorage.removeItem('recovery_dialog_shown');
        await refetchMeetings();
        if (!othersLeft) router.push(`/meeting-details?id=${encodeURIComponent(recoveredId)}`);
        return;
      }

      if (result.success) {
        toast.success(t('recoveredSuccessTitle'), {
          description: result.audioRecoveryStatus?.status === 'success'
            ? t('recoveredWithAudio')
            : t('recoveredNoAudio'),
          action: result.meetingId ? {
            label: t('viewMeetingAction'),
            onClick: () => {
              router.push(`/meeting-details?id=${result.meetingId}`);
            }
          } : undefined,
          duration: 10000,
        });

        // Refresh sidebar to show the newly recovered meeting
        await refetchMeetings();

        // Nothing left: a recording interrupted later this session opens the
        // dialog by itself again.
        if (!othersLeft) sessionStorage.removeItem('recovery_dialog_shown');

        // Auto-navigate after a short delay, once nothing else waits
        if (result.meetingId && !othersLeft) {
          setTimeout(() => {
            router.push(`/meeting-details?id=${result.meetingId}`);
          }, 2000);
        }
      }
    } catch (error) {
      toastFailure(t('recoveryFailedTitle'), 'transcript-recovery', error);
      throw error;
    }
  };

  // The dialog opens by itself once per session; after that the line above
  // the transcript opens it again for as long as anything waits.
  const handleDialogClose = () => {
    setShowRecoveryDialog(false);
    if (recoverableMeetings.length === 0) {
      sessionStorage.removeItem('recovery_dialog_shown');
    }
  };

  // Computed values using global status
  const isProcessingStop = status === RecordingStatus.PROCESSING_TRANSCRIPTS || isProcessing;

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3, ease: 'easeOut' }}
      className="flex flex-col h-full bg-gray-50"
    >
      {/* All Modals supported*/}
      <SettingsModals
        modals={modals}
        messages={messages}
        onClose={hideModal}
      />

      {/* Recovery Dialog */}
      <TranscriptRecovery
        isOpen={showRecoveryDialog}
        onClose={handleDialogClose}
        recoverableMeetings={recoverableMeetings}
        onRecover={handleRecovery}
        onDelete={deleteRecoverableMeeting}
        onOpenWithOtherKey={openWithOtherKey}
        onLoadPreview={loadMeetingTranscripts}
      />
      {recoverableMeetings.length > 0 && !showRecoveryDialog && !recordingState.isRecording && (
        <div
          role="status"
          className="flex items-center justify-between gap-3 border-b border-[var(--af-border)] bg-[var(--af-bg)] px-4 py-2 text-sm"
        >
          <span>{t('interruptedRecordings', { count: recoverableMeetings.length })}</span>
          <button
            type="button"
            onClick={() => setShowRecoveryDialog(true)}
            className="rounded-md border border-[var(--af-border)] px-3 py-1 font-medium hover:bg-[var(--af-hover)]"
          >
            {t('interruptedRecordingsOpen')}
          </button>
        </div>
      )}
      <div className="flex flex-1 overflow-hidden">
        <TranscriptPanel
          isProcessingStop={isProcessingStop}
          isStopping={isStopping}
          showModal={showModal}
          notesOpen={showNotes}
          notesCount={recordingNotes.notes.length}
          onToggleNotes={toggleNotes}
        />

        {showNotes && (
          <aside
            aria-label={tn('title')}
            className="flex shrink-0 flex-col border-l border-[var(--af-border)] bg-[var(--af-bg)]"
            style={{ width: NOTES_WIDTH }}
          >
            <NotesEditor source={recordingNotes} live />
          </aside>
        )}

        {/* Recording controls - hidden only behind the status overlays. Without a
            usable microphone the start button stays, disabled, with the reason. */}
        {status !== RecordingStatus.PROCESSING_TRANSCRIPTS &&
          status !== RecordingStatus.SAVING && (
            <div className="fixed bottom-12 left-0 right-0 z-30 pointer-events-none">
              <div
                className="flex justify-center px-4 transition-[margin] duration-300 pointer-events-none"
                // The controls stay over the transcript, never over the notes.
                style={{ marginLeft: 'var(--sidebar-offset)', marginRight: showNotes ? NOTES_WIDTH : undefined }}
              >
                <div className="w-full max-w-[750px] flex justify-center pointer-events-auto">
                  <div className="flex w-full min-w-0 items-center justify-center">
                    <RecordingControls
                      isRecording={recordingState.isRecording}
                      onRecordingStop={(callApi = true) => handleRecordingStop(callApi)}
                      onRecordingStart={handleRecordingStart}
                      onTranscriptReceived={() => { }} // Not actually used by RecordingControls
                      onStopInitiated={() => setIsStopping(true)}
                      onTranscriptionError={(message) => {
                        showModal('errorAlert', message);
                      }}
                      isRecordingDisabled={isRecordingDisabled}
                      isParentProcessing={isProcessingStop}
                      selectedDevices={selectedDevices}
                      meetingName={meetingTitle}
                    />
                  </div>
                </div>
              </div>
            </div>
          )}

        {/* Status Overlays - Processing and Saving */}
        <StatusOverlays
          isProcessing={status === RecordingStatus.PROCESSING_TRANSCRIPTS && !recordingState.isRecording}
          isSaving={status === RecordingStatus.SAVING}
        />
      </div>
    </motion.div>
  );
}

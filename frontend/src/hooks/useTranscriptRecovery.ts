/**
 * useTranscriptRecovery Hook
 *
 * Orchestrates transcript recovery operations for interrupted meetings.
 * Detects, previews and recovers recordings whose transcript never reached the
 * database. The transcript comes from the backend journal of each recording
 * (sealed with the archive key); nothing is kept in the window.
 */

import { useState, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  type MeetingMetadata,
  type StoredTranscript,
  type UnsavedRecording,
  toMeetingMetadata,
  toStoredTranscripts,
} from '@/lib/unsaved-recordings';
import { storageService } from '@/services/storageService';
import { applyPinnedSummaryLanguageToMeeting } from '@/lib/summary-language-preferences';
import { toast } from 'sonner';
import { useTranslations } from 'next-intl';

interface AudioRecoveryStatus {
  status: string; // "success" | "partial" | "failed" | "none"
  chunk_count: number;
  estimated_duration_seconds: number;
  audio_file_path?: string;
  message: string;
}

export interface RecoveryResult {
  success: boolean;
  audioRecoveryStatus?: AudioRecoveryStatus | null;
  meetingId?: string;
  /**
   * Set when only the audio was restored: the meeting is empty until the
   * post-call processing recognises this folder again.
   */
  transcribeFolder?: string;
}

interface RestoredRecording {
  meetingId: string;
  folderPath: string;
}

export interface UseTranscriptRecoveryReturn {
  recoverableMeetings: MeetingMetadata[];
  isLoading: boolean;
  isRecovering: boolean;
  checkForRecoverableTranscripts: () => Promise<void>;
  recoverMeeting: (meetingId: string) => Promise<RecoveryResult>;
  loadMeetingTranscripts: (meetingId: string) => Promise<StoredTranscript[]>;
  deleteRecoverableMeeting: (meetingId: string) => Promise<void>;
}

export function useTranscriptRecovery(): UseTranscriptRecoveryReturn {
  const t = useTranslations('recording');
  const [recoverableMeetings, setRecoverableMeetings] = useState<MeetingMetadata[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isRecovering, setIsRecovering] = useState(false);
  /** The journals as last read, by folder. */
  const unsaved = useRef<Map<string, UnsavedRecording>>(new Map());

  /**
   * Look for recordings whose transcript was never saved
   */
  const checkForRecoverableTranscripts = useCallback(async () => {
    setIsLoading(true);
    try {
      const found = await invoke<UnsavedRecording[]>('list_unsaved_recordings');
      unsaved.current = new Map(found.map((recording) => [recording.folderPath, recording]));
      const recentMeetings = found.map(toMeetingMetadata);

      // Verify audio checkpoint availability for each meeting
      const meetingsWithAudioStatus = await Promise.all(
        recentMeetings.map(async (meeting) => {
          // Listed for its audio in the first place.
          if (meeting.audioOnly) return meeting;
          if (meeting.folderPath) {
            try {
              const hasAudio = await invoke<boolean>('has_audio_checkpoints', {
                meetingFolder: meeting.folderPath
              });

              // If no audio files, clear folderPath to show "No audio" in UI
              return {
                ...meeting,
                folderPath: hasAudio ? meeting.folderPath : undefined
              };
            } catch (error) {
              console.warn('Failed to check audio for meeting:', error);
              // On error, assume no audio to be safe
              return { ...meeting, folderPath: undefined };
            }
          }
          return meeting;
        })
      );


      setRecoverableMeetings(meetingsWithAudioStatus);
    } catch (error) {
      console.error('Failed to check for recoverable transcripts:', error);
      setRecoverableMeetings([]);
    } finally {
      setIsLoading(false);
    }
  }, []);

  /**
   * Load transcripts for preview
   */
  const loadMeetingTranscripts = useCallback(async (meetingId: string): Promise<StoredTranscript[]> => {
    try {
      const recording = unsaved.current.get(meetingId);
      return recording ? toStoredTranscripts(recording) : [];
    } catch (error) {
      console.error('Failed to load meeting transcripts:', error);
      return [];
    }
  }, []);

  /**
   * Save an unsaved recording to the database
   */
  const recoverMeeting = useCallback(async (meetingId: string): Promise<RecoveryResult> => {
    setIsRecovering(true);
    try {
      // 1. Load meeting metadata
      const recording = unsaved.current.get(meetingId);
      const metadata = recording ? toMeetingMetadata(recording) : null;
      if (!recording || !metadata) {
        throw new Error('Meeting metadata not found');
      }

      // Only audio survived: the backend makes an empty meeting of the folder
      // and the caller has it recognised again.
      if (recording.audioOnly) {
        const restored = await invoke<RestoredRecording>('restore_recording_without_meeting', {
          folderPath: recording.folderPath,
          title: t('restoredRecordingTitle', {
            date: new Date(metadata.startTime).toLocaleString(),
          }),
        });
        unsaved.current.delete(meetingId);
        setRecoverableMeetings(prev => prev.filter(m => m.meetingId !== meetingId));
        return {
          success: true,
          meetingId: restored.meetingId,
          transcribeFolder: restored.folderPath,
        };
      }

      // 2. Load all transcripts
      const transcripts = await loadMeetingTranscripts(meetingId);
      if (transcripts.length === 0) {
        throw new Error('No transcripts found for this meeting');
      }

      // 3. The journal lives in the recording folder, so the folder is known.
      const folderPath: string | undefined = meetingId;

      // 4. Attempt audio recovery if folder path exists
      let audioRecoveryStatus: AudioRecoveryStatus | null = null;
      if (folderPath) {
        try {
          audioRecoveryStatus = await invoke<AudioRecoveryStatus>(
            'recover_audio_from_checkpoints',
            { meetingFolder: folderPath, sampleRate: 48000 }
          );
        } catch (error) {
          console.error('Audio recovery failed:', error);
          audioRecoveryStatus = {
            status: 'failed',
            chunk_count: 0,
            estimated_duration_seconds: 0,
            message: error instanceof Error ? error.message : 'Unknown error'
          };
        }
      } else {
        audioRecoveryStatus = {
          status: 'none',
          chunk_count: 0,
          estimated_duration_seconds: 0,
          message: 'No folder path available'
        };
      }

      // 5. Convert StoredTranscripts to the format expected by storageService
      const formattedTranscripts = transcripts.map((t, index) => ({
        id: t.id?.toString() || `${Date.now()}-${index}`,
        text: t.text,
        timestamp: t.timestamp,
        sequence_id: t.sequenceId || index,
        is_partial: false,
        confidence: t.confidence,
        audio_start_time: t.audio_start_time,
        audio_end_time: t.audio_end_time,
        duration: t.duration,
        speaker: t.speaker,
      }));

      // 6. Save to backend database using existing save utilities
      const saveResponse = await storageService.saveMeeting(
        metadata.title,
        formattedTranscripts,
        folderPath ?? null,
        new Date(metadata.startTime).toISOString()
      );

      const savedMeetingId = saveResponse.meeting_id;

      try {
        await applyPinnedSummaryLanguageToMeeting(savedMeetingId);
      } catch (error) {
        console.warn('Failed to apply pinned summary language to recovered meeting:', error);
        toast.warning(t('summaryLanguageApplyFailedTitle'), {
          description: t('summaryLanguageApplyFailedDescriptionRecovered'),
        });
      }

      // 7. Saving removed the journal: the recording is in the database now.


      // 8. Clean up checkpoint files
      if (folderPath) {
        try {
          await invoke('cleanup_checkpoints', { meetingFolder: folderPath });
        } catch (error) {
          // Non-fatal - don't fail recovery if cleanup fails
          console.warn('Checkpoint cleanup failed (non-fatal):', error);
        }
      }

      // 9. Remove from recoverable list
      setRecoverableMeetings(prev => prev.filter(m => m.meetingId !== meetingId));

      return {
        success: true,
        audioRecoveryStatus,
        meetingId: savedMeetingId
      };
    } catch (error) {
      console.error('Failed to recover meeting:', error);
      throw error;
    } finally {
      setIsRecovering(false);
    }
  }, [loadMeetingTranscripts, t]);

  /**
   * Delete a recoverable meeting
   */
  const deleteRecoverableMeeting = useCallback(async (meetingId: string): Promise<void> => {
    try {
      await invoke('discard_unsaved_transcript', { folderPath: meetingId });
      unsaved.current.delete(meetingId);
      setRecoverableMeetings(prev => prev.filter(m => m.meetingId !== meetingId));
    } catch (error) {
      console.error('Failed to delete meeting:', error);
      throw error;
    }
  }, []);

  return {
    recoverableMeetings,
    isLoading,
    isRecovering,
    checkForRecoverableTranscripts,
    recoverMeeting,
    loadMeetingTranscripts,
    deleteRecoverableMeeting
  };
}

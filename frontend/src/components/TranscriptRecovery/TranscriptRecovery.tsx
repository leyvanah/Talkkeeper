/**
 * TranscriptRecovery Component
 *
 * Modal dialog for recovering interrupted meetings from IndexedDB.
 * Displays recoverable meetings, allows preview, and enables recovery or deletion.
 */

import React, { useState, useEffect } from 'react';
import { AlertCircle, AudioLines, CheckCircle2, Clock, EyeOff, FileText, XCircle } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Alert, AlertDescription } from '@/components/ui/alert';
import type { AudioOnly, MeetingMetadata, StoredTranscript } from '@/lib/unsaved-recordings';
import { cn } from '@/lib/utils';
import { useFormatter, useTranslations } from 'next-intl';
import { toast } from 'sonner';

interface TranscriptRecoveryProps {
  isOpen: boolean;
  onClose: () => void;
  recoverableMeetings: MeetingMetadata[];
  onRecover: (meetingId: string) => Promise<any>;
  onDelete: (meetingId: string) => Promise<void>;
  onLoadPreview: (meetingId: string) => Promise<StoredTranscript[]>;
}

export function TranscriptRecovery({
  isOpen,
  onClose,
  recoverableMeetings,
  onRecover,
  onDelete,
  onLoadPreview,
}: TranscriptRecoveryProps) {
  const t = useTranslations('recording');
  const tCommon = useTranslations('common');
  const format = useFormatter();
  const [selectedMeetingId, setSelectedMeetingId] = useState<string | null>(null);
  const [previewTranscripts, setPreviewTranscripts] = useState<StoredTranscript[]>([]);
  const [isLoadingPreview, setIsLoadingPreview] = useState(false);
  const [isRecovering, setIsRecovering] = useState(false);
  const [isDeleting, setIsDeleting] = useState(false);
  // "Don't recover" asks first, inside the dialog: window.confirm is not
  // something the application window can be relied on to show.
  const [confirmingDismiss, setConfirmingDismiss] = useState(false);

  // Reset selection when dialog opens
  useEffect(() => {
    if (isOpen) {
      setConfirmingDismiss(false);
      setSelectedMeetingId(null);
      setPreviewTranscripts([]);
    }
  }, [isOpen]);

  // Auto-select first meeting if available
  useEffect(() => {
    if (isOpen && recoverableMeetings.length > 0 && !selectedMeetingId) {
      handleMeetingSelect(recoverableMeetings[0].meetingId);
    }
  }, [isOpen, recoverableMeetings]);

  const handleMeetingSelect = async (meetingId: string) => {
    setSelectedMeetingId(meetingId);
    setConfirmingDismiss(false);
    // A recording with only audio has no text to preview.
    if (recoverableMeetings.find(m => m.meetingId === meetingId)?.audioOnly) {
      setPreviewTranscripts([]);
      return;
    }
    setIsLoadingPreview(true);

    try {
      const transcripts = await onLoadPreview(meetingId);
      // Limit to first 10 for preview
      setPreviewTranscripts(transcripts.slice(0, 10));
    } catch (error) {
      console.error('Failed to load preview:', error);
      setPreviewTranscripts([]);
    } finally {
      setIsLoadingPreview(false);
    }
  };

  const handleRecover = async () => {
    if (!selectedMeetingId) return;

    setIsRecovering(true);
    try {
      const result = await onRecover(selectedMeetingId);
      console.log('Recovery successful:', result);
      onClose();
    } catch (error) {
      // The caller has already said what went wrong.
      console.error('Recovery failed:', error);
    } finally {
      setIsRecovering(false);
    }
  };

  const handleDelete = async () => {
    if (!selectedMeetingId) return;
    setConfirmingDismiss(false);
    setIsDeleting(true);
    try {
      await onDelete(selectedMeetingId);
      setSelectedMeetingId(null);
      setPreviewTranscripts([]);
    } catch (error) {
      console.error('Delete failed:', error);
      toast.error(t('deleteFailedAlert'), {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setIsDeleting(false);
    }
  };

  const selectedMeeting = recoverableMeetings.find(m => m.meetingId === selectedMeetingId);
  const cannotRecover = selectedMeeting?.audioOnly?.readable === false;

  const startOf = (meeting: MeetingMetadata) =>
    format.dateTime(new Date(meeting.startTime), { dateStyle: 'medium', timeStyle: 'short' });
  // Two recordings without a name must still be told apart: by when they began.
  const titleOf = (meeting: MeetingMetadata) =>
    meeting.title.trim() || t('recordingFrom', { date: startOf(meeting) });

  /** Size and, when known, length of a recording that has only audio. */
  const describeAudio = (audio: AudioOnly) => {
    const megabytes = audio.sizeBytes / (1024 * 1024);
    const parts = [t('audioOnlySize', { size: megabytes < 10 ? Math.round(megabytes * 10) / 10 : Math.round(megabytes) })];
    if (audio.durationSeconds) {
      parts.push(t('audioOnlyDuration', { minutes: Math.max(1, Math.round(audio.durationSeconds / 60)) }));
    }
    return parts.join(' · ');
  };

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent
        className="max-w-4xl h-[80vh] flex flex-col p-0"
        // Focusing the first recording on open drew a ring around it that
        // looked like a second selection.
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <DialogHeader className="px-6 pt-6">
          <DialogTitle className="text-2xl">{t('recoverInterruptedMeetingsTitle')}</DialogTitle>
          <DialogDescription>
            {t('recoverDescription', { count: recoverableMeetings.length })}
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 flex gap-4 px-6 pb-6 overflow-hidden">
          {/* Meeting List */}
          <div className="w-1/3 flex flex-col">
            <h3 className="text-sm font-medium mb-2">{t('interruptedMeetingsHeading')}</h3>
            <ScrollArea className="flex-1 border rounded-lg">
              <div className="p-2 space-y-2">
                {recoverableMeetings.map((meeting) => (
                  <button
                    key={meeting.meetingId}
                    onClick={() => handleMeetingSelect(meeting.meetingId)}
                    aria-pressed={selectedMeetingId === meeting.meetingId}
                    className={cn(
                      'w-full text-left p-3 rounded-lg border-2 transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring',
                      selectedMeetingId === meeting.meetingId
                        ? 'bg-accent border-foreground/70'
                        : 'hover:bg-muted border-transparent'
                    )}
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex-1 min-w-0">
                        <p className="font-medium text-sm truncate">{titleOf(meeting)}</p>
                        <p className="text-xs text-muted-foreground flex items-center gap-1 mt-1">
                          <Clock className="w-3 h-3" />
                          {meeting.title.trim() ? startOf(meeting) : format.relativeTime(new Date(meeting.startTime))}
                        </p>
                        {meeting.audioOnly ? (
                          <p className="text-xs text-muted-foreground flex items-center gap-1 mt-1">
                            <AudioLines className="w-3 h-3" />
                            {t('audioOnlyBadge')} · {describeAudio(meeting.audioOnly)}
                          </p>
                        ) : (
                          <p className="text-xs text-muted-foreground flex items-center gap-1 mt-1">
                            <FileText className="w-3 h-3" />
                            {t('transcriptCount', { count: meeting.transcriptCount })}
                          </p>
                        )}
                      </div>
                      {meeting.audioOnly?.readable === false ? (
                        <span title={t('audioOnlyUnreadable')}>
                          <XCircle className="w-4 h-4 text-red-500 flex-shrink-0" />
                        </span>
                      ) : meeting.folderPath ? (
                        <span title={t('audioAvailable')}>
                          <CheckCircle2 className="w-4 h-4 text-green-500 flex-shrink-0" />
                        </span>
                      ) : (
                        <span title={t('noAudioTitle')}>
                          <AlertCircle className="w-4 h-4 text-yellow-500 flex-shrink-0" />
                        </span>
                      )}
                    </div>
                  </button>
                ))}
              </div>
            </ScrollArea>
          </div>

          {/* Preview Panel */}
          <div className="flex-1 flex flex-col">
            <h3 className="text-sm font-medium mb-2">{t('previewHeading')}</h3>
            <div className="flex-1 border rounded-lg overflow-hidden flex flex-col">
              {selectedMeeting ? (
                <>
                  {/* Meeting Info */}
                  <div className="p-4 border-b bg-muted/50">
                    <h4 className="font-semibold">{titleOf(selectedMeeting)}</h4>
                    <p className="text-sm text-muted-foreground mt-1">
                      {t('startedAt', { date: startOf(selectedMeeting) })}
                    </p>
                    <div className="flex items-center gap-4 mt-2 text-sm">
                      {selectedMeeting.audioOnly ? (
                        <span className="flex items-center gap-1">
                          <AudioLines className="w-4 h-4" />
                          {t('audioOnlyBadge')} · {describeAudio(selectedMeeting.audioOnly)}
                        </span>
                      ) : (
                        <span className="flex items-center gap-1">
                          <FileText className="w-4 h-4" />
                          {t('transcriptCount', { count: selectedMeeting.transcriptCount })}
                        </span>
                      )}
                      {cannotRecover ? (
                        <span className="flex items-center gap-1 text-red-600">
                          <XCircle className="w-4 h-4" />
                          {t('audioOnlyUnreadable')}
                        </span>
                      ) : selectedMeeting.folderPath ? (
                        <span className="flex items-center gap-1 text-green-600">
                          <CheckCircle2 className="w-4 h-4" />
                          {t('audioAvailable')}
                        </span>
                      ) : (
                        <span className="flex items-center gap-1 text-yellow-600">
                          <AlertCircle className="w-4 h-4" />
                          {t('noAudioTitle')}
                        </span>
                      )}
                    </div>
                  </div>

                  {/* Transcript Preview */}
                  <ScrollArea className="flex-1 p-4">
                    {selectedMeeting.audioOnly ? (
                      <Alert variant={cannotRecover ? 'destructive' : 'default'}>
                        <AlertDescription>
                          {cannotRecover ? t('audioOnlyUnreadableExplanation') : t('audioOnlyExplanation')}
                        </AlertDescription>
                      </Alert>
                    ) : isLoadingPreview ? (
                      <div className="flex items-center justify-center h-full text-muted-foreground">
                        {t('loadingPreview')}
                      </div>
                    ) : previewTranscripts.length > 0 ? (
                      <div className="space-y-3">
                        <Alert>
                          <AlertDescription>
                            {t('showingFirstSegments', { shown: previewTranscripts.length, total: selectedMeeting.transcriptCount })}
                          </AlertDescription>
                        </Alert>
                        {previewTranscripts.map((transcript, index) => {
                          // Handle different timestamp formats
                          const getTimestamp = () => {
                            if (!transcript.timestamp) return '--:--';
                            try {
                              const date = new Date(transcript.timestamp);
                              if (isNaN(date.getTime())) {
                                // If timestamp is invalid, try audio_start_time
                                if (transcript.audio_start_time !== undefined) {
                                  const totalSecs = Math.floor(transcript.audio_start_time);
                                  const mins = Math.floor(totalSecs / 60);
                                  const secs = totalSecs % 60;
                                  return `${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
                                }
                                return '--:--';
                              }
                              return date.toLocaleTimeString();
                            } catch {
                              return '--:--';
                            }
                          };

                          return (
                            <div key={index} className="text-sm">
                              <span className="text-muted-foreground">[{getTimestamp()}]</span>{' '}
                              <span>{transcript.text}</span>
                            </div>
                          );
                        })}
                        {selectedMeeting.transcriptCount > 10 && (
                          <p className="text-sm text-muted-foreground italic">
                            {t('andMoreTranscripts', { count: selectedMeeting.transcriptCount - 10 })}
                          </p>
                        )}
                      </div>
                    ) : (
                      <div className="flex items-center justify-center h-full text-muted-foreground">
                        {t('noTranscriptsToPreview')}
                      </div>
                    )}
                  </ScrollArea>
                </>
              ) : (
                <div className="flex items-center justify-center h-full text-muted-foreground">
                  {t('selectMeetingToPreview')}
                </div>
              )}
            </div>
          </div>
        </div>

        {confirmingDismiss && selectedMeeting ? (
          <DialogFooter className="px-6 pb-6 items-center">
            <p className="text-sm text-muted-foreground mr-auto">
              {t('confirmDeleteRecoverableMeeting')}
            </p>
            <Button variant="outline" onClick={() => setConfirmingDismiss(false)}>
              {tCommon('cancel')}
            </Button>
            <Button onClick={handleDelete}>
              <EyeOff className="w-4 h-4 mr-2" />
              {t('dontRestoreConfirm')}
            </Button>
          </DialogFooter>
        ) : (
        <DialogFooter className="px-6 pb-6">
          <Button
            variant="outline"
            onClick={onClose}
            disabled={isRecovering || isDeleting}
          >
            {tCommon('cancel')}
          </Button>
          <Button
            variant="outline"
            onClick={() => setConfirmingDismiss(true)}
            disabled={!selectedMeetingId || isRecovering || isDeleting}
          >
            {isDeleting ? (
              <>
                <XCircle className="w-4 h-4 mr-2 animate-spin" />
                {t('deletingEllipsis')}
              </>
            ) : (
              <>
                <EyeOff className="w-4 h-4 mr-2" />
                {t('dontRestoreButton')}
              </>
            )}
          </Button>
          <Button
            onClick={handleRecover}
            disabled={!selectedMeetingId || cannotRecover || isRecovering || isDeleting}
          >
            {isRecovering ? (
              <>
                <CheckCircle2 className="w-4 h-4 mr-2 animate-spin" />
                {t('recoveringEllipsis')}
              </>
            ) : (
              <>
                <CheckCircle2 className="w-4 h-4 mr-2" />
                {t('recoverButton')}
              </>
            )}
          </Button>
        </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  );
}

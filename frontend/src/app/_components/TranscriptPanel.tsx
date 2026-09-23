/**
 * Transcript panel for the LIVE recording screen (`app/page.tsx`).
 *
 * ⚠️ There are TWO components named `TranscriptPanel`. This is the live one.
 * The meeting-details screen uses `components/MeetingDetails/TranscriptPanel.tsx`.
 * Editing the wrong file is a common trap — the change compiles and appears to
 * do nothing, because the screen you are looking at renders the other one.
 *
 * Transcripts arrive from `TranscriptContext`, which maps the Rust event's
 * `source` field onto `speaker`. The converter below must keep `speaker` or
 * live speaker labels silently disappear.
 */

import { VirtualizedTranscriptView } from '@/components/VirtualizedTranscriptView';
import { PermissionWarning } from '@/components/PermissionWarning';
import { RecordingClientSelector } from '@/components/RecordingClientSelector';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { AudioLines, Copy, GlobeIcon } from 'lucide-react';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useConfig } from '@/contexts/ConfigContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { usePermissionCheck } from '@/hooks/usePermissionCheck';
import { ModalType } from '@/hooks/useModalState';
import { useIsLinux } from '@/hooks/usePlatform';
import { useEffect, useMemo, useState } from 'react';
import { useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

/**
 * Whether the recording on now is only sound: live text turned off, or the
 * engine busy with a transcription job when it started. Asked of the backend
 * when a recording is found running, and told by its start event.
 */
function useSoundOnlyRecording(isRecording: boolean): boolean {
  const [soundOnly, setSoundOnly] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listen<{ liveTranscription?: boolean }>('recording-started', (event) => {
      setSoundOnly(event.payload?.liveTranscription === false);
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!isRecording) {
      setSoundOnly(false);
      return;
    }
    let cancelled = false;
    void invoke<boolean>('recording_live_transcription')
      .then((live) => !cancelled && setSoundOnly(!live))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [isRecording]);

  return soundOnly;
}

/**
 * TranscriptPanel Component
 *
 * Displays transcript content with controls for copying and language settings.
 * Uses TranscriptContext, ConfigContext, and RecordingStateContext internally.
 */

interface TranscriptPanelProps {
  // indicates stop-processing state for transcripts; derived from backend statuses.
  isProcessingStop: boolean;
  isStopping: boolean;
  showModal: (name: ModalType, message?: string) => void;
}

export function TranscriptPanel({
  isProcessingStop,
  isStopping,
  showModal
}: TranscriptPanelProps) {
  const t = useTranslations('recording');

  // Contexts
  const { transcripts, transcriptContainerRef, copyTranscript } = useTranscripts();
  const { transcriptModelConfig } = useConfig();
  const { isRecording, isPaused } = useRecordingState();
  const { requestPermissions, isChecking, hasSystemAudio, hasMicrophone } = usePermissionCheck();
  const isLinux = useIsLinux();
  const soundOnly = useSoundOnlyRecording(isRecording);

  // Convert transcripts to segments for virtualized view
  const segments = useMemo(() =>
    transcripts.map(t => ({
      id: t.id,
      timestamp: t.audio_start_time ?? 0,
      endTime: t.audio_end_time,
      text: t.text,
      confidence: t.confidence,
      speaker: t.speaker,
    })),
    [transcripts]
  );

  return (
    <div ref={transcriptContainerRef} className="w-full border-r border-gray-200 bg-white flex flex-col overflow-y-auto">
      {/* Title area - Sticky header */}
      <div className="sticky top-0 z-10 bg-white p-4 border-gray-200">
        <div className="flex flex-col space-y-3">
          <div className="flex  flex-col space-y-2">
            <div className="flex flex-wrap justify-center items-center gap-2">
              <RecordingClientSelector />
              <ButtonGroup>
                {transcripts?.length > 0 && (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={copyTranscript}
                    title={t('copyTranscript')}
                  >
                    <Copy />
                    <span className='hidden md:inline'>
                      {t('copy')}
                    </span>
                  </Button>
                )}
                {transcriptModelConfig.provider === "localWhisper" &&
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => showModal('languageSettings')}
                    title={t('language')}
                  >
                    <GlobeIcon />
                    <span className='hidden md:inline'>
                      {t('language')}
                    </span>
                  </Button>
                }
              </ButtonGroup>
            </div>
          </div>
        </div>
      </div>

      {/* Permission Warning - Not needed on Linux */}
      {!isRecording && !isChecking && !isLinux && (
        <div className="flex justify-center px-4 pt-4">
          <PermissionWarning
            hasMicrophone={hasMicrophone}
            hasSystemAudio={hasSystemAudio}
            onRecheck={requestPermissions}
            isRechecking={isChecking}
          />
        </div>
      )}

      {/* Transcript content.
          Extra bottom padding while recording (incl. paused) so the last bubble
          sits above the fixed floating control bar — without it, pause freezes
          auto-scroll and the latest line ends up under the bar. */}
      <div
        className={isRecording ? 'pb-40' : 'pb-20'}
        style={isRecording ? { scrollPaddingBottom: '10rem' } : undefined}
      >
        {soundOnly && (
          <div className="flex flex-col items-center gap-2 px-6 pt-16 text-center">
            <AudioLines size={22} className="text-[var(--af-text-3)]" />
            <p className="text-sm font-medium text-[var(--af-text-2)]">{t('soundOnlyTitle')}</p>
            <p className="max-w-sm text-sm text-[var(--af-text-3)]">{t('soundOnlyBody')}</p>
          </div>
        )}
        <div className={`justify-center ${soundOnly ? 'hidden' : 'flex'}`}>
          <div className="w-2/3 max-w-[750px]">
            <VirtualizedTranscriptView
              segments={segments}
              isRecording={isRecording}
              isPaused={isPaused}
              isProcessing={isProcessingStop}
              isStopping={isStopping}
              enableStreaming={isRecording && !isPaused}
              showConfidence={true}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

'use client';

/**
 * In-app recording widget shown on the main screen (app/page.tsx).
 *
 * Two visual states, both styled to mirror the floating compact bar (minibar):
 *  - Idle: red mic button + "Start Recording/Ready", a Mic row (selected device
 *    name) and a System row that shows green "Detected" / red "Not detected".
 *    Detection comes from usePermissionCheck (5s poll) — NOT from the audio
 *    stream, since the webview cannot see system audio (see CLAUDE.md).
 *  - Recording: status dot + timer, stacked Mic/System level meters
 *    (LiveAudioVisualizer with `fill`), and Pause / Stop / shrink controls.
 *
 * Wiring:
 *  - Start/stop go through Tauri commands (invoke) and RecordingStateContext.
 *  - Live audio meters are fed by the Rust `recording-audio-levels` event
 *    (pre-mix, per-source) — the webview cannot capture system audio, so the
 *    meters must be Rust-driven.
 *  - The "shrink" control hands off to the minibar window; the minibar's Stop
 *    is driven from Rust (minibar::stop_recording_from_minibar), because
 *    cross-window emit/listen to the minibar webview is unreliable.
 */

import { invoke } from '@tauri-apps/api/core';

import { useCallback, useEffect, useState } from 'react';
import { Play, Pause, Square, Mic, MicOff, Volume2, VolumeX, AlertCircle, X, Minimize2, Loader2 } from 'lucide-react';
import { LiveAudioVisualizer } from './LiveAudioVisualizer';
import { ProcessRequest, SummaryResponse } from '@/types/summary';
import { listen } from '@tauri-apps/api/event';
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { usePermissionCheck } from '@/hooks/usePermissionCheck';
import { useTranslations } from 'next-intl';

interface RecordingControlsProps {
  isRecording: boolean;
  barHeights: string[];
  onRecordingStop: (callApi?: boolean) => void;
  onRecordingStart: () => void;
  onTranscriptReceived: (summary: SummaryResponse) => void;
  onTranscriptionError?: (message: string) => void;
  onStopInitiated?: () => void; // Called immediately when stop button is clicked
  isRecordingDisabled: boolean;
  isParentProcessing: boolean;
  selectedDevices?: {
    micDevice: string | null;
    systemDevice: string | null;
  };
  meetingName?: string;
}

/** Removes a Tauri listener without letting a failed removal take the app
 *  down. Nothing useful can be done about it — the listener is either gone
 *  already or will go with the window — and an exception here surfaces as an
 *  unhandled runtime error over the whole interface. */
function safelyUnsubscribe(unsubscribe: (() => void) | undefined) {
  if (typeof unsubscribe !== 'function') return;
  const warn = (error: unknown) => console.warn('Failed to remove a recording event listener:', error);
  try {
    // Tauri's unlisten is asynchronous: a failure inside it comes back as a
    // rejected promise, which try/catch alone would let through.
    const removal = unsubscribe() as unknown;
    if (removal && typeof (removal as PromiseLike<unknown>).then === 'function') {
      void Promise.resolve(removal).catch(warn);
    }
  } catch (error) {
    warn(error);
  }
}

export const RecordingControls: React.FC<RecordingControlsProps> = ({
  isRecording,
  barHeights,
  onRecordingStop,
  onRecordingStart,
  onTranscriptReceived,
  onTranscriptionError,
  onStopInitiated,
  isRecordingDisabled,
  isParentProcessing,
  selectedDevices,
  meetingName,
}) => {
  const t = useTranslations('recording');
  // Use global recording state context for pause state (syncs with tray operations)
  const recordingState = useRecordingState();
  const isPaused = recordingState.isPaused;
  const isMicrophoneMuted = recordingState.isMicrophoneMuted;
  const isSystemAudioMuted = recordingState.isSystemAudioMuted;
  // Phase text published by useRecordingStart ("Preparing transcription
  // model…", "Starting audio capture…") so the wait is explained rather than
  // just being a dead button.
  const startupMessage = recordingState.statusMessage;

  // For the idle bar: the selected mic name, and a live-ish system-audio check.
  const { hasSystemAudio, checkPermissions } = usePermissionCheck();
  const micName = selectedDevices?.micDevice?.trim() || t('defaultMicrophone');
  useEffect(() => {
    if (isRecording) return;
    const id = setInterval(() => { checkPermissions(); }, 5000);
    return () => clearInterval(id);
    // checkPermissions is stable enough for a polling interval; re-subscribing
    // on every render would defeat the interval.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isRecording]);

  const [showPlayback, setShowPlayback] = useState(false);
  const [recordingPath, setRecordingPath] = useState<string | null>(null);
  const [transcript, setTranscript] = useState<string>('');
  const [isProcessing, setIsProcessing] = useState(false);
  const [isStarting, setIsStarting] = useState(false);
  const [isStopping, setIsStopping] = useState(false);
  const [isPausing, setIsPausing] = useState(false);
  const [isResuming, setIsResuming] = useState(false);
  const [isChangingMicrophoneMute, setIsChangingMicrophoneMute] = useState(false);
  const [isChangingSystemAudioMute, setIsChangingSystemAudioMute] = useState(false);
  const MIN_RECORDING_DURATION = 2000; // 2 seconds minimum recording time
  const [transcriptionErrors, setTranscriptionErrors] = useState(0);
  const [isValidatingModel, setIsValidatingModel] = useState(false);
  const [speechDetected, setSpeechDetected] = useState(false);
  const [deviceError, setDeviceError] = useState<{ title: string, message: string } | null>(null);
  // Coach-mark above the minimize button after recording starts.
  const [showCompactTip, setShowCompactTip] = useState(false);

  const currentTime = 0;
  const duration = 0;
  const isPlaying = false;
  const progress = 0;

  const formatTime = (time: number) => {
    const minutes = Math.floor(time / 60);
    const seconds = Math.floor(time % 60);
    return `${minutes}:${seconds.toString().padStart(2, '0')}`;
  };

  // Elapsed timer for the in-app bar, mirroring the floating compact bar.
  const elapsedSeconds = Math.max(0, Math.floor(recordingState.recordingDuration ?? 0));
  const formatElapsed = (totalSeconds: number) => {
    const h = Math.floor(totalSeconds / 3600);
    const m = Math.floor((totalSeconds % 3600) / 60);
    const s = Math.floor(totalSeconds % 60);
    return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  };

  // Tip above the minimize control when recording starts.
  useEffect(() => {
    const onTip = () => {
      setShowCompactTip(true);
      window.setTimeout(() => setShowCompactTip(false), 12000);
    };
    window.addEventListener('show-compact-mode-tip', onTip);
    return () => window.removeEventListener('show-compact-mode-tip', onTip);
  }, []);

  useEffect(() => {
    if (!isRecording) setShowCompactTip(false);
  }, [isRecording]);

  useEffect(() => {
    const checkTauri = async () => {
      try {
        const result = await invoke('is_recording');
        console.log('Tauri is initialized and ready, is_recording result:', result);
      } catch (error) {
        console.error('Tauri initialization error:', error);
        alert(t('initFailedAlert'));
      }
    };
    checkTauri();
  }, []);

  const handleStartRecording = useCallback(async () => {
    if (isStarting || isValidatingModel) return;
    console.log('Starting recording...');
    console.log('Selected devices:', selectedDevices);
    console.log('Meeting name:', meetingName);
    console.log('Current isRecording state:', isRecording);

    setShowPlayback(false);
    setTranscript(''); // Clear any previous transcript
    setSpeechDetected(false); // Reset speech detection on new recording

    // Mark the button busy for the whole start sequence. This was previously
    // only ever read, never set, so the spinner never appeared and the button
    // looked unresponsive during model load.
    setIsStarting(true);

    try {
      // Call the validation callback which will:
      // 1. Check if model is ready
      // 2. Show appropriate toast/modal
      // 3. Call backend if valid
      // 4. Update UI state
      await onRecordingStart();
    } catch (error) {
      console.error('Failed to start recording:', error);
      console.error('Error details:', {
        message: error instanceof Error ? error.message : String(error),
        name: error instanceof Error ? error.name : 'Unknown',
        stack: error instanceof Error ? error.stack : undefined
      });

      // Parse error message to provide user-friendly feedback
      const errorMsg = error instanceof Error ? error.message : String(error);

      // Check for device-related errors
      if (errorMsg.includes('microphone') || errorMsg.includes('mic') || errorMsg.includes('input')) {
        setDeviceError({
          title: t('micNotAvailableTitle'),
          message: t('micNotAvailableMessage')
        });
      } else if (errorMsg.includes('system audio') || errorMsg.includes('speaker') || errorMsg.includes('output')) {
        setDeviceError({
          title: t('systemAudioNotAvailableTitle'),
          message: t('systemAudioNotAvailableMessage')
        });
      } else if (errorMsg.includes('permission')) {
        setDeviceError({
          title: t('permissionRequiredTitle'),
          message: t('permissionRequiredMessage')
        });
      } else {
        setDeviceError({
          title: t('recordingFailedTitle'),
          message: t('recordingFailedMessage')
        });
      }
    } finally {
      setIsStarting(false);
    }
  }, [onRecordingStart, isStarting, isValidatingModel, selectedDevices, meetingName, isRecording]);

  const stopRecordingAction = useCallback(async () => {
    console.log('Executing stop recording...');
    try {
      setIsProcessing(true);
      // Portable build: recordings save into the program's install-local data
      // root (same directory returned for the database), not %APPDATA%.
      const dataDir = await invoke<string>('get_database_directory');
      const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
      const savePath = `${dataDir}/recording-${timestamp}.wav`;
      console.log('Saving recording to:', savePath);
      console.log('About to call stop_recording command');
      const didStop = await invoke<boolean>('stop_recording', {
        args: {
          save_path: savePath
        }
      });
      console.log('stop_recording command completed successfully:', didStop);
      if (!didStop) {
        setIsProcessing(false);
        return;
      }
      setRecordingPath(savePath);
      // setShowPlayback(true);
      setIsProcessing(false);
      // Native stop emits one main-window-only completion event. The global
      // post-processing provider handles transcript drain/save/navigation for
      // every stop origin, so do not start a second frontend owner here.
    } catch (error) {
      console.error('Failed to stop recording:', error);
      if (error instanceof Error) {
        console.error('Error details:', {
          message: error.message,
          name: error.name,
          stack: error.stack,
        });
        if (error.message.includes('No recording in progress')) {
          return;
        }
      } else if (typeof error === 'string' && error.includes('No recording in progress')) {
        return;
      } else if (error && typeof error === 'object' && 'toString' in error) {
        if (error.toString().includes('No recording in progress')) {
          return;
        }
      }
      setIsProcessing(false);
      onRecordingStop(false);
    } finally {
      setIsStopping(false);
    }
  }, [onRecordingStop]);

  const handleStopRecording = useCallback(async () => {
    console.log('handleStopRecording called - isRecording:', isRecording, 'isStarting:', isStarting, 'isStopping:', isStopping);
    if (!isRecording || isStarting || isStopping) {
      console.log('Early return from handleStopRecording due to state check');
      return;
    }

    console.log('Stopping recording...');

    // Notify parent immediately (for UI state updates)
    onStopInitiated?.();

    setIsStopping(true);

    // Immediately trigger the stop action
    await stopRecordingAction();
  }, [isRecording, isStarting, isStopping, stopRecordingAction, onStopInitiated]);

  const handlePauseRecording = useCallback(async () => {
    if (!isRecording || isPaused || isPausing) return;

    console.log('Pausing recording...');
    setIsPausing(true);

    try {
      await invoke('pause_recording');
      // isPaused state now managed by RecordingStateContext via events
      console.log('Recording paused successfully');
    } catch (error) {
      console.error('Failed to pause recording:', error);
      alert(t('pauseFailedAlert'));
    } finally {
      setIsPausing(false);
    }
  }, [isRecording, isPaused, isPausing]);

  const handleResumeRecording = useCallback(async () => {
    if (!isRecording || !isPaused || isResuming) return;

    console.log('Resuming recording...');
    setIsResuming(true);

    try {
      await invoke('resume_recording');
      // isPaused state now managed by RecordingStateContext via events
      console.log('Recording resumed successfully');
    } catch (error) {
      console.error('Failed to resume recording:', error);
      alert(t('resumeFailedAlert'));
    } finally {
      setIsResuming(false);
    }
  }, [isRecording, isPaused, isResuming]);

  const handleMicrophoneMute = useCallback(async () => {
    if (!isRecording || isStopping || isChangingMicrophoneMute || isChangingSystemAudioMute) return;

    setIsChangingMicrophoneMute(true);
    try {
      await invoke<boolean>('set_microphone_muted', { muted: !isMicrophoneMuted });
    } catch (error) {
      console.error('Failed to change microphone mute state:', error);
    } finally {
      setIsChangingMicrophoneMute(false);
    }
  }, [isChangingMicrophoneMute, isChangingSystemAudioMute, isMicrophoneMuted, isRecording, isStopping]);

  const handleSystemAudioMute = useCallback(async () => {
    if (!isRecording || isStopping || isChangingMicrophoneMute || isChangingSystemAudioMute) return;

    setIsChangingSystemAudioMute(true);
    try {
      await invoke<boolean>('set_system_audio_muted', { muted: !isSystemAudioMuted });
    } catch (error) {
      console.error('Failed to change system audio mute state:', error);
    } finally {
      setIsChangingSystemAudioMute(false);
    }
  }, [isChangingMicrophoneMute, isChangingSystemAudioMute, isRecording, isStopping, isSystemAudioMuted]);

  // Collapse the full window down to the floating compact bar. Mirrors the
  // bar's expand button so the two are one control surface in two sizes; the
  // current duration seeds the bar so its timer continues rather than resets.
  const collapseToBar = useCallback(() => {
    const elapsed = Math.max(0, Math.floor(recordingState.recordingDuration ?? 0));
    invoke('enter_compact_mode', { elapsedSeconds: elapsed }).catch((e) =>
      console.error('Failed to enter compact mode:', e)
    );
  }, [recordingState.recordingDuration]);

  useEffect(() => {
    return () => {
      // Cleanup on unmount if needed
    };
  }, []);

  useEffect(() => {
    console.log('Setting up recording event listeners');
    let unsubscribes: (() => void)[] = [];
    // Subscribing is asynchronous and unmounting is not, so the cleanup can
    // run while the three listens are still in flight. Without this flag
    // those listeners were registered after the component was gone and
    // never removed, and a later unsubscribe reached into an entry Tauri no
    // longer had — the `handlerId` of undefined the owner kept seeing when
    // stopping a recording.
    let cancelled = false;

    const setupListeners = async () => {
      try {
        // Transcript error listener - handles both regular and actionable errors
        const transcriptErrorUnsubscribe = await listen('transcript-error', (event) => {
          console.log('transcript-error event received:', event);
          console.error('Transcription error received:', event.payload);
          const errorMessage = event.payload as string;
          console.log('Tracked transcription error:', errorMessage);

          setTranscriptionErrors(prev => {
            const newCount = prev + 1;
            console.log('Transcription error count incremented:', newCount);
            return newCount;
          });
          setIsProcessing(false);
          console.log('Calling onRecordingStop(false) due to transcript error');
          onRecordingStop(false);
          if (onTranscriptionError) {
            onTranscriptionError(errorMessage);
          }
        });

        // Transcription error listener - handles structured error objects with actionable flag
        const transcriptionErrorUnsubscribe = await listen('transcription-error', (event) => {
          console.log('transcription-error event received:', event);
          console.error('Transcription error received:', event.payload);

          let errorMessage: string;
          let isActionable = false;

          if (typeof event.payload === 'object' && event.payload !== null) {
            const payload = event.payload as { error: string, userMessage: string, actionable: boolean };
            errorMessage = payload.userMessage || payload.error;
            isActionable = payload.actionable || false;
          } else {
            errorMessage = String(event.payload);
          }
          console.log('Tracked transcription error:', errorMessage);

          setTranscriptionErrors(prev => {
            const newCount = prev + 1;
            console.log('Transcription error count incremented:', newCount);
            return newCount;
          });
          setIsProcessing(false);
          console.log('Calling onRecordingStop(false) due to transcription error');
          onRecordingStop(false);

          // For actionable errors (like model loading failures), the main page will handle showing the model selector
          // For regular errors, they are handled by useModalState global listener which shows a toast
          // We don't want to show a modal (via onTranscriptionError) AND a toast, so we skip the callback here
          /* if (onTranscriptionError && !isActionable) {
            onTranscriptionError(errorMessage);
          } */
        });

        // Pause/Resume events are now handled by RecordingStateContext
        // No need for duplicate listeners here

        // Speech detected listener - for UX feedback when VAD detects speech
        const speechDetectedUnsubscribe = await listen('speech-detected', (event) => {
          console.log('speech-detected event received:', event);
          setSpeechDetected(true);
        });

        if (cancelled) {
          // Unmounted while we were subscribing: undo it here, because the
          // cleanup has already been and gone.
          [
            transcriptErrorUnsubscribe,
            transcriptionErrorUnsubscribe,
            speechDetectedUnsubscribe
          ].forEach(safelyUnsubscribe);
          return;
        }

        unsubscribes = [
          transcriptErrorUnsubscribe,
          transcriptionErrorUnsubscribe,
          speechDetectedUnsubscribe
        ];
        console.log('Recording event listeners set up successfully');
      } catch (error) {
        console.error('Failed to set up recording event listeners:', error);
      }
    };

    setupListeners();

    return () => {
      console.log('Cleaning up recording event listeners');
      cancelled = true;
      // Emptied before unsubscribing, so a second cleanup cannot remove the
      // same listener twice.
      const pending = unsubscribes;
      unsubscribes = [];
      pending.forEach(safelyUnsubscribe);
    };
  }, [onRecordingStop, onTranscriptionError]);

  // The theme's own surface, so the bar belongs to every theme alike.
  const panel =
    'flex items-center rounded-2xl border border-[var(--af-border)] bg-[var(--af-panel)] text-[var(--af-text)] shadow-[var(--af-shadow-md)]';

  return (
    <TooltipProvider>
      <div className="flex flex-col items-center space-y-2">
        {isProcessing && !isParentProcessing ? (
          <div className={`${panel} gap-2 px-4 py-3`}>
            <Loader2 size={16} className="animate-spin text-[var(--af-text-3)]" />
            <span className="text-sm text-[var(--af-text-2)]">{t('processingRecording')}</span>
          </div>
        ) : showPlayback ? (
          <div className={`${panel} gap-3 px-4 py-3`}>
            <button
              onClick={handleStartRecording}
              className="flex h-10 w-10 items-center justify-center rounded-full bg-red-500 text-white transition-colors hover:bg-red-600"
            >
              <Mic size={16} />
            </button>
            <div className="flex items-center gap-2 text-sm text-[var(--af-text-2)]">
              <span className="min-w-[40px] tabular-nums">{formatTime(currentTime)}</span>
              <div className="relative h-1 w-24 rounded-full bg-[var(--af-border)]">
                <div className="absolute h-full rounded-full bg-[var(--af-accent)]" style={{ width: `${progress}%` }} />
              </div>
              <span className="min-w-[40px] tabular-nums">{formatTime(duration)}</span>
            </div>
          </div>
        ) : !isRecording ? (
          // At rest there is no panel: the button, what it does, and one quiet
          // line saying what will be recorded.
          <div className="flex flex-col items-center gap-2">
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  onClick={() => {
                    handleStartRecording();
                  }}
                  disabled={isStarting || isProcessing || isRecordingDisabled || isValidatingModel}
                  aria-label={t('startRecording')}
                  className={`relative flex h-14 w-14 shrink-0 items-center justify-center rounded-full text-white transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-red-500 focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--af-bg)] ${
                    isStarting || isProcessing || isValidatingModel ? 'bg-red-500/50' : 'bg-red-500 hover:bg-red-600'
                  }`}
                >
                  {isStarting || isValidatingModel ? <Loader2 size={22} className="animate-spin" /> : <Mic size={24} />}
                </button>
              </TooltipTrigger>
              <TooltipContent>
                <p>{t('startRecordingTooltip')}</p>
              </TooltipContent>
            </Tooltip>

            <div className="text-sm font-medium text-[var(--af-text)]">
              {isStarting || isValidatingModel ? startupMessage || t('starting') : t('startRecording')}
            </div>

            <div className="flex max-w-[28rem] items-center gap-1.5 text-xs text-[var(--af-text-3)]">
              <span className="min-w-0 truncate" title={t('microphoneTitle', { micName })}>{micName}</span>
              <span aria-hidden>·</span>
              <span
                className={`inline-flex shrink-0 items-center gap-1.5 ${hasSystemAudio ? '' : 'text-red-400'}`}
                title={hasSystemAudio ? t('detected') : t('notDetected')}
              >
                {t('systemAudio')}
                <span className={`h-1.5 w-1.5 rounded-full ${hasSystemAudio ? 'bg-emerald-500' : 'bg-red-500'}`} />
                {!hasSystemAudio && <span>{t('notDetected')}</span>}
              </span>
            </div>
          </div>
        ) : (
          // While recording: one line on the theme's own surface, readable
          // over the live transcript it floats above.
          <div className={`${panel} w-[600px] max-w-full gap-4 px-4 py-2.5`}>
            <div className="flex shrink-0 items-center gap-2.5">
              <span
                className={`h-2.5 w-2.5 rounded-full ${isPaused ? 'bg-orange-400' : 'animate-pulse bg-red-500'}`}
                aria-hidden
              />
              <div className="leading-tight">
                <div className="text-[15px] font-medium tabular-nums">{formatElapsed(elapsedSeconds)}</div>
                <div className={`text-[11px] ${isPaused ? 'text-orange-400' : 'text-[var(--af-text-3)]'}`}>
                  {isStopping ? t('stopping') : isPaused ? t('paused') : t('recordingLabel')}
                </div>
              </div>
            </div>

            {/* Live input levels (Rust-driven, per source), each with its mute. */}
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              {([
                ['mic', isMicrophoneMuted, handleMicrophoneMute, t('muteMicrophone'), t('unmuteMicrophone'), Mic, MicOff],
                ['system', isSystemAudioMuted, handleSystemAudioMute, t('muteSystemAudio'), t('unmuteSystemAudio'), Volume2, VolumeX],
              ] as const).map(([source, muted, toggle, muteLabel, unmuteLabel, On, Off]) => (
                <div key={source} className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={toggle}
                    disabled={isStopping || isChangingMicrophoneMute || isChangingSystemAudioMute}
                    title={muted ? unmuteLabel : muteLabel}
                    aria-label={muted ? unmuteLabel : muteLabel}
                    aria-pressed={muted}
                    className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-md transition-colors disabled:opacity-40 ${
                      muted
                        ? 'bg-orange-500/15 text-orange-400 hover:bg-orange-500/25'
                        : 'text-[var(--af-text-3)] hover:bg-[var(--af-hover)] hover:text-[var(--af-text)]'
                    }`}
                  >
                    {muted ? <Off size={13} /> : <On size={13} />}
                  </button>
                  <LiveAudioVisualizer
                    active={isRecording && !isPaused && !muted}
                    source={source}
                    fill
                    bars={28}
                    className="flex-1"
                  />
                </div>
              ))}
            </div>

            <div className="flex shrink-0 items-center gap-1.5">
              <button
                onClick={() => {
                  if (isPaused) {
                    handleResumeRecording();
                  } else {
                    handlePauseRecording();
                  }
                }}
                disabled={isPausing || isResuming || isStopping || isChangingMicrophoneMute || isChangingSystemAudioMute}
                title={isPaused ? t('resumeRecordingTooltip') : t('pauseRecordingTooltip')}
                className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-[var(--af-border)] px-2.5 text-xs text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] disabled:opacity-40"
              >
                {isPaused ? <Play size={13} /> : <Pause size={13} />}
                {isPaused ? t('resume') : t('pause')}
              </button>

              <button
                onClick={() => {
                  handleStopRecording();
                }}
                disabled={isStopping || isPausing || isResuming || isChangingMicrophoneMute || isChangingSystemAudioMute}
                title={t('stopRecordingTooltip')}
                className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-red-500/40 px-2.5 text-xs text-red-500 transition-colors hover:bg-red-500/10 disabled:opacity-40"
              >
                <Square size={11} fill="currentColor" />
                {t('stop')}
              </button>

              <div className="relative">
                {showCompactTip && (
                  <div
                    role="dialog"
                    aria-label={t('shrinkToFloatingBarAria')}
                    className="absolute bottom-[calc(100%+12px)] right-0 z-50 w-[260px] rounded-xl border border-[var(--af-border)] bg-[var(--af-panel)] px-3.5 py-3 text-left shadow-lg"
                  >
                    {/* Caret pointing at the minimize button */}
                    <span
                      aria-hidden
                      className="absolute -bottom-1.5 right-3 h-3 w-3 rotate-45 border-b border-r border-[var(--af-border)] bg-[var(--af-panel)]"
                    />
                    <button
                      type="button"
                      onClick={() => setShowCompactTip(false)}
                      className="absolute right-2 top-2 rounded p-0.5 text-[var(--af-text-3)] hover:text-[var(--af-text)]"
                      aria-label={t('dismiss')}
                    >
                      <X size={14} />
                    </button>
                    <div className="pr-5 text-sm font-semibold text-[var(--af-text)]">
                      {t('youAreRecording')}
                    </div>
                    <p className="mt-1 text-xs leading-relaxed text-[var(--af-text-2)]">
                      {t('shrinkTipDescription')}
                    </p>
                    <button
                      type="button"
                      onClick={() => {
                        setShowCompactTip(false);
                        collapseToBar();
                      }}
                      className="mt-3 w-full rounded-lg bg-[var(--af-accent)] px-3 py-1.5 text-xs font-semibold text-[var(--af-accent-contrast)] transition-[filter] hover:brightness-110"
                    >
                      {t('shrinkToBar')}
                    </button>
                  </div>
                )}
                <button
                  onClick={() => {
                    setShowCompactTip(false);
                    collapseToBar();
                  }}
                  disabled={isStopping}
                  title={t('shrinkToFloatingBarAria')}
                  aria-label={t('shrinkToFloatingBarAria')}
                  className={`flex h-8 w-8 items-center justify-center rounded-lg text-[var(--af-text-3)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] disabled:opacity-40 ${
                    showCompactTip ? 'ring-2 ring-[var(--af-accent)]/40' : ''
                  }`}
                >
                  <Minimize2 size={14} />
                </button>
              </div>
            </div>
          </div>
        )}

        {/* Show validation status only */}
        {isValidatingModel && (
          <div className="mt-2 text-center text-xs text-[var(--af-text-3)]">
            {t('validatingSpeechRecognition')}
          </div>
        )}

        {/* Device error alert */}
        {deviceError && (
          <Alert variant="destructive" className="mt-4 border-red-300 bg-red-50">
            <AlertCircle className="h-5 w-5 text-red-600" />
            <button
              onClick={() => setDeviceError(null)}
              className="absolute right-3 top-3 text-red-600 hover:text-red-800 transition-colors"
              aria-label={t('closeAlert')}
            >
              <X className="h-4 w-4" />
            </button>
            <AlertTitle className="text-red-800 font-semibold mb-2">
              {deviceError.title}
            </AlertTitle>
            <AlertDescription className="text-red-700">
              {deviceError.message.split('\n').map((line, i) => (
                <div key={i} className={i > 0 ? 'ml-2' : ''}>
                  {line}
                </div>
              ))}
            </AlertDescription>
          </Alert>
        )}

        {/* {showPlayback && recordingPath && (
        <div className="text-sm text-gray-600 px-4">
          Recording saved to: {recordingPath}
        </div>
      )} */}
      </div>
    </TooltipProvider>
  );
};

'use client';

import React, { createContext, useContext, useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { recordingService } from '@/services/recordingService';

/**
 * Recording state synchronized with backend
 * This context provides a single source of truth for recording state
 * that automatically syncs with the Rust backend, solving:
 * 1. Page refresh desync (backend recording but UI shows stopped)
 * 2. Pause state visibility across components
 * 3. Comprehensive state for future features (reconnection, etc.)
 */

// Recording lifecycle status enum
export enum RecordingStatus {
  IDLE = 'idle',                          // Not recording
  STARTING = 'starting',                  // Initiating recording
  RECORDING = 'recording',                // Active recording
  STOPPING = 'stopping',                  // Stop initiated, waiting for backend
  PROCESSING_TRANSCRIPTS = 'processing',  // Transcription completion wait
  SAVING = 'saving',                      // Saving to database
  COMPLETED = 'completed',                // Successfully saved
  ERROR = 'error'                         // Error occurred
}

interface RecordingState {
  isRecording: boolean;           // Is a recording session active
  isPaused: boolean;              // Is the recording paused
  isMicrophoneMuted: boolean;     // Is only the microphone silenced
  isSystemAudioMuted: boolean;    // Is only system audio silenced
  isActive: boolean;              // Is actively recording (recording && !paused)

  // NEW: Lifecycle status
  status: RecordingStatus;
  statusMessage?: string;  // Optional message for current status
}

/**
 * The recording's clock, kept apart from the state above: it changes on every
 * poll, and in the shared context it re-rendered every consumer - the page,
 * the transcript, the sidebar - twice a second for a whole recording, which
 * cost a video call running beside the app its processor time.
 */
export interface RecordingClock {
  recordingDuration: number | null;  // Total duration including pauses
  activeDuration: number | null;     // Active recording time (excluding pauses)
}

interface RecordingStateContextType extends RecordingState {
  // NEW: Setters for status management
  setStatus: (status: RecordingStatus, message?: string) => void;
  /** The clock as of now, without subscribing to its ticks. */
  readClock: () => RecordingClock;

  // Computed helpers (derived from status)
  isStopping: boolean;
  isProcessing: boolean;
  isSaving: boolean;
}

const RecordingStateContext = createContext<RecordingStateContextType | null>(null);

const STOPPED_CLOCK: RecordingClock = { recordingDuration: null, activeDuration: null };
const RecordingClockContext = createContext<RecordingClock>(STOPPED_CLOCK);

/** Re-renders on every clock tick: only for components that show the time. */
export const useRecordingClock = () => useContext(RecordingClockContext);

export const useRecordingState = () => {
  const context = useContext(RecordingStateContext);
  if (!context) {
    throw new Error('useRecordingState must be used within a RecordingStateProvider');
  }
  return context;
};

export function RecordingStateProvider({ children }: { children: React.ReactNode }) {
  const [state, setState] = useState<RecordingState>({
    isRecording: false,
    isPaused: false,
    isMicrophoneMuted: false,
    isSystemAudioMuted: false,
    isActive: false,
    status: RecordingStatus.IDLE,  // NEW: Initialize with IDLE status
    statusMessage: undefined,       // NEW: No message initially
  });
  const [clock, setClock] = useState<RecordingClock>(STOPPED_CLOCK);
  const clockRef = useRef<RecordingClock>(STOPPED_CLOCK);
  const updateClock = useCallback((next: RecordingClock) => {
    clockRef.current = next;
    setClock(prev =>
      prev.recordingDuration === next.recordingDuration && prev.activeDuration === next.activeDuration
        ? prev
        : next);
  }, []);
  const readClock = useCallback(() => clockRef.current, []);

  const pollingIntervalRef = useRef<NodeJS.Timeout | null>(null);

  // NEW: Status setter with logging
  const setStatus = useCallback((status: RecordingStatus, message?: string) => {
    console.log(`[RecordingState] Status: ${state.status} → ${status}`, message || '');

    setState(prev => ({
      ...prev,
      status,
      statusMessage: message,
    }));
  }, [state.status, state.isRecording, state.isPaused]);

  /**
   * Sync recording state with backend
   * Called on mount (fixes refresh desync) and periodically while recording
   */
  const syncWithBackend = async () => {
    try {
      const backendState = await recordingService.getRecordingState();

      // An unchanged state keeps its identity, so a poll that only moved
      // the clock re-renders nobody but the clock's readers.
      setState(prev =>
        prev.isRecording === backendState.is_recording &&
        prev.isPaused === backendState.is_paused &&
        prev.isMicrophoneMuted === backendState.is_microphone_muted &&
        prev.isSystemAudioMuted === backendState.is_system_audio_muted &&
        prev.isActive === backendState.is_active
          ? prev
          : {
            ...prev,
            isRecording: backendState.is_recording,
            isPaused: backendState.is_paused,
            isMicrophoneMuted: backendState.is_microphone_muted,
            isSystemAudioMuted: backendState.is_system_audio_muted,
            isActive: backendState.is_active,
          });
      updateClock({
        recordingDuration: backendState.recording_duration,
        activeDuration: backendState.active_duration,
      });
    } catch (error) {
      console.error('[RecordingStateContext] Failed to sync with backend:', error);
      // Don't update state on error - keep current state
    }
  };

  /**
   * Start polling backend state (called when recording starts)
   */
  const startPolling = () => {
    if (pollingIntervalRef.current) {
      clearInterval(pollingIntervalRef.current);
    }

    console.log('[RecordingStateContext] Starting state polling (500ms interval)');
    pollingIntervalRef.current = setInterval(syncWithBackend, 500);
  };

  /**
   * Stop polling backend state (called when recording stops)
   */
  const stopPolling = () => {
    if (pollingIntervalRef.current) {
      console.log('[RecordingStateContext] Stopping state polling');
      clearInterval(pollingIntervalRef.current);
      pollingIntervalRef.current = null;
    }
  };

  /**
   * Set up event listeners for backend state changes
   */
  useEffect(() => {
    console.log('[RecordingStateContext] Setting up event listeners');
    const unsubscribers: (() => void)[] = [];

    const setupListeners = async () => {
      try {
        // Recording started
        const unlistenStarted = await recordingService.onRecordingStarted(() => {
          console.log('[RecordingStateContext] Recording started event');
          setState(prev => ({
            ...prev,
            isRecording: true,
            isPaused: false,
            isMicrophoneMuted: false,
            isSystemAudioMuted: false,
            isActive: true,
            status: RecordingStatus.RECORDING,  // NEW: Set status to RECORDING
          }));
          startPolling();
        });
        unsubscribers.push(unlistenStarted);

        // Recording stopped
        const unlistenStopped = await recordingService.onRecordingStopped((payload) => {
          console.log('[RecordingStateContext] Recording stopped event:', payload);
          setState(prev => {
            // Set status to STOPPING if not already in stop flow
            // This ensures smooth UI transition for tray/keyboard stops
            const newStatus = [
              RecordingStatus.STOPPING,
              RecordingStatus.PROCESSING_TRANSCRIPTS,
              RecordingStatus.SAVING
            ].includes(prev.status)
              ? prev.status  // Already in stop flow
              : RecordingStatus.STOPPING;  // New stop, transition smoothly

            return {
              ...prev,
              status: newStatus,
              statusMessage: newStatus === RecordingStatus.STOPPING ? 'Stopping recording...' : prev.statusMessage,
              isRecording: false,
              isPaused: false,
              isMicrophoneMuted: false,
              isSystemAudioMuted: false,
              isActive: false,
            };
          });
          updateClock(STOPPED_CLOCK);
          stopPolling();
        });
        unsubscribers.push(unlistenStopped);

        // Recording paused
        const unlistenPaused = await recordingService.onRecordingPaused(() => {
          console.log('[RecordingStateContext] Recording paused event');
          setState(prev => ({
            ...prev,
            isPaused: true,
            isActive: false,
          }));
        });
        unsubscribers.push(unlistenPaused);

        // Recording resumed
        const unlistenResumed = await recordingService.onRecordingResumed(() => {
          console.log('[RecordingStateContext] Recording resumed event');
          setState(prev => ({
            ...prev,
            isPaused: false,
            isActive: true,
          }));
        });
        unsubscribers.push(unlistenResumed);

        const unlistenMicrophoneMute = await recordingService.onMicrophoneMuteChanged(({ muted }) => {
          console.log('[RecordingStateContext] Microphone mute changed:', muted);
          setState(prev => ({
            ...prev,
            isMicrophoneMuted: muted,
          }));
        });
        unsubscribers.push(unlistenMicrophoneMute);

        const unlistenSystemAudioMute = await recordingService.onSystemAudioMuteChanged(({ muted }) => {
          console.log('[RecordingStateContext] System audio mute changed:', muted);
          setState(prev => ({
            ...prev,
            isSystemAudioMuted: muted,
          }));
        });
        unsubscribers.push(unlistenSystemAudioMute);

        console.log('[RecordingStateContext] Event listeners set up successfully');
      } catch (error) {
        console.error('[RecordingStateContext] Failed to set up event listeners:', error);
      }
    };

    setupListeners();

    return () => {
      console.log('[RecordingStateContext] Cleaning up event listeners');
      unsubscribers.forEach(unsub => unsub());
      stopPolling();
    };
  }, []);

  /**
   * Initial sync on mount - CRITICAL for fixing refresh desync bug
   * If backend is recording but UI state is false, this will correct it
   */
  useEffect(() => {
    console.log('[RecordingStateContext] Initial mount - syncing with backend');
    syncWithBackend();
  }, []);

  // NEW: Computed helpers from status
  const contextValue = useMemo(() => ({
    ...state,
    setStatus,
    readClock,
    isStopping: state.status === RecordingStatus.STOPPING,
    isProcessing: state.status === RecordingStatus.PROCESSING_TRANSCRIPTS,
    isSaving: state.status === RecordingStatus.SAVING,
  }), [state, setStatus, readClock]);

  return (
    <RecordingStateContext.Provider value={contextValue}>
      <RecordingClockContext.Provider value={clock}>
        {children}
      </RecordingClockContext.Provider>
    </RecordingStateContext.Provider>
  );
}

'use client';

/**
 * Compact recording bar — the entire UI of the frameless `minibar` window.
 *
 * Shown while recording so the user can keep an eye on the timer and input
 * levels, and pause/stop, without the full window taking over their screen.
 * One narrow line in the app's own theme; it can also be put away entirely,
 * and the recording goes on — the tray icon brings the window back.
 *
 * Deliberately does NOT duplicate the stop logic. Rust owns native finalization
 * and emits the completion event that makes the main window save and navigate.
 */

import { useCallback, useEffect, useState } from 'react';
import { useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import { EyeOff, Maximize2, Mic, MicOff, Pause, Play, Square, Volume2, VolumeX } from 'lucide-react';
import { LiveAudioVisualizer } from '@/components/LiveAudioVisualizer';
import { recordingService } from '@/services/recordingService';
import { applyAppTheme, getSavedAppTheme } from '@/lib/app-theme';

function formatElapsed(totalSeconds: number): string {
  const h = Math.floor(totalSeconds / 3600);
  const m = Math.floor((totalSeconds % 3600) / 60);
  const s = Math.floor(totalSeconds % 60);
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
}

export default function MiniBarPage() {
  const t = useTranslations('app');
  const tr = useTranslations('recording');
  const [elapsed, setElapsed] = useState(0);
  const [isPaused, setIsPaused] = useState(false);
  const [isMicMuted, setIsMicMuted] = useState(false);
  const [isChangingMicMute, setIsChangingMicMute] = useState(false);
  const [isSystemMuted, setIsSystemMuted] = useState(false);
  const [isChangingSystemMute, setIsChangingSystemMute] = useState(false);
  // Once stopping begins the timer must freeze immediately, even though the
  // bar stays up until the recording is actually finalised (see below).
  const [isStopping, setIsStopping] = useState(false);

  // The window is transparent; the page must not paint a background over it.
  // Same origin as the main window, so the saved theme is readable here.
  useEffect(() => {
    document.documentElement.classList.add('minibar-window');
    applyAppTheme(getSavedAppTheme());
    return () => document.documentElement.classList.remove('minibar-window');
  }, []);

  // Rust's RecordingState uses a monotonic Instant. Reading that duration keeps
  // this separate webview aligned through creation delays, pauses, and duplicate
  // minimize events instead of accumulating drift in a local +1 counter.
  useEffect(() => {
    let mounted = true;
    const syncFromNative = async () => {
      try {
        const state = await recordingService.getRecordingState();
        if (!mounted) return;
        const duration = state.active_duration ?? state.recording_duration;
        if (duration !== null) {
          setElapsed(Math.max(0, Math.floor(duration)));
        }
        setIsPaused(state.is_paused);
        setIsMicMuted(state.is_microphone_muted);
        setIsSystemMuted(state.is_system_audio_muted);
      } catch (error) {
        console.error('Compact bar: failed to sync recording state', error);
      }
    };
    void syncFromNative();
    const id = window.setInterval(syncFromNative, 500);
    return () => {
      mounted = false;
      window.clearInterval(id);
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void recordingService.onSystemAudioMuteChanged(({ muted }) => {
      if (!disposed) setIsSystemMuted(muted);
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    }).catch((error) => {
      console.error('Compact bar: failed to listen for system audio mute changes', error);
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void recordingService.onMicrophoneMuteChanged(({ muted }) => {
      if (!disposed) setIsMicMuted(muted);
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    }).catch((error) => {
      console.error('Compact bar: failed to listen for microphone mute changes', error);
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const togglePause = useCallback(async () => {
    try {
      await invoke(isPaused ? 'resume_recording' : 'pause_recording');
    } catch (e) {
      console.error('Compact bar: pause/resume failed', e);
    }
  }, [isPaused]);

  const toggleMicMute = useCallback(async () => {
    if (isChangingMicMute || isChangingSystemMute || isStopping) return;
    setIsChangingMicMute(true);
    try {
      await recordingService.setMicrophoneMuted(!isMicMuted);
    } catch (error) {
      console.error('Compact bar: microphone mute failed', error);
    } finally {
      setIsChangingMicMute(false);
    }
  }, [isChangingMicMute, isChangingSystemMute, isMicMuted, isStopping]);

  const toggleSystemMute = useCallback(async () => {
    if (isChangingMicMute || isChangingSystemMute || isStopping) return;
    setIsChangingSystemMute(true);
    try {
      await recordingService.setSystemAudioMuted(!isSystemMuted);
    } catch (error) {
      console.error('Compact bar: system audio mute failed', error);
    } finally {
      setIsChangingSystemMute(false);
    }
  }, [isChangingMicMute, isChangingSystemMute, isStopping, isSystemMuted]);

  const expand = useCallback(() => {
    invoke('exit_compact_mode').catch((e) => console.error(e));
  }, []);

  const hide = useCallback(() => {
    invoke('hide_compact_bar').catch((e) => console.error('Compact bar: hide failed', e));
  }, []);

  const stop = useCallback(async () => {
    // Rust closes this native window as soon as it claims shutdown. Do not wait
    // for a frontend event from another webview to remove the bar.
    setIsStopping(true);
    try {
      const didStop = await invoke<boolean>('stop_recording_from_minibar');
      if (!didStop) {
        console.log('Compact bar: native shutdown was already owned');
      }
    } catch (e) {
      console.error('Compact bar: stop failed', e);
      setIsStopping(false);
    }
  }, []);

  const busy = isStopping || isChangingMicMute || isChangingSystemMute;
  const icon =
    'flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] disabled:opacity-40';
  const sources = [
    ['mic', isMicMuted, toggleMicMute, tr('muteMicrophone'), tr('unmuteMicrophone'), Mic, MicOff],
    ['system', isSystemMuted, toggleSystemMute, tr('muteSystemAudio'), tr('unmuteSystemAudio'), Volume2, VolumeX],
  ] as const;

  return (
    <div
      data-tauri-drag-region
      className="flex h-screen w-screen select-none items-center gap-2 rounded-full border border-[var(--af-border-strong)] bg-[var(--af-panel)] pl-3.5 pr-1.5 text-[var(--af-text)]"
    >
      {/* Status and timer; the words are in the tooltip, the colour says it. */}
      <div
        data-tauri-drag-region
        className="flex shrink-0 items-center gap-2"
        title={isStopping ? tr('stopping') : isPaused ? tr('paused') : tr('recordingLabel')}
      >
        <span
          data-tauri-drag-region
          className={`h-2.5 w-2.5 rounded-full ${
            isStopping ? 'bg-[var(--af-text-3)]' : isPaused ? 'bg-orange-400' : 'animate-pulse bg-red-500'
          }`}
        />
        <span data-tauri-drag-region className="text-sm font-medium tabular-nums">
          {formatElapsed(elapsed)}
        </span>
      </div>

      <span data-tauri-drag-region className="mx-0.5 h-5 w-px shrink-0 bg-[var(--af-border)]" />

      {/* Each source: its mute, and a small level beside it. */}
      {sources.map(([source, muted, toggle, muteLabel, unmuteLabel, On, Off]) => (
        <div key={source} className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            onClick={toggle}
            disabled={busy}
            title={muted ? unmuteLabel : muteLabel}
            aria-label={muted ? unmuteLabel : muteLabel}
            aria-pressed={muted}
            className={`${icon} ${muted ? 'bg-orange-500/15 !text-orange-400' : ''}`}
          >
            {muted ? <Off size={14} /> : <On size={14} />}
          </button>
          <LiveAudioVisualizer active={!isPaused && !isStopping && !muted} source={source} bars={6} />
        </div>
      ))}

      <div data-tauri-drag-region className="flex-1" />

      <button
        type="button"
        onClick={togglePause}
        disabled={busy}
        title={isPaused ? t('minibarResume') : t('minibarPause')}
        aria-label={isPaused ? t('minibarResume') : t('minibarPause')}
        className={icon}
      >
        {isPaused ? <Play size={14} /> : <Pause size={14} />}
      </button>
      <button
        type="button"
        onClick={stop}
        disabled={busy}
        title={t('minibarStopTitle')}
        aria-label={t('minibarStopTitle')}
        className={`${icon} !text-red-500 hover:!bg-red-500/10`}
      >
        <Square size={12} fill="currentColor" />
      </button>

      <span data-tauri-drag-region className="mx-0.5 h-5 w-px shrink-0 bg-[var(--af-border)]" />

      <button
        type="button"
        onClick={expand}
        disabled={busy}
        title={t('minibarExpandTitle')}
        aria-label={t('minibarExpandTitle')}
        className={icon}
      >
        <Maximize2 size={13} />
      </button>
      <button
        type="button"
        onClick={hide}
        disabled={isStopping}
        title={t('minibarHideTitle')}
        aria-label={t('minibarHideTitle')}
        className={icon}
      >
        <EyeOff size={14} />
      </button>
    </div>
  );
}

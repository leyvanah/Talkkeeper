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
 *
 * Dragged to the top of a screen it docks there (Rust decides, see
 * `minibar_dock.rs`), hangs from the edge as a tab and slides away while the
 * cursor is elsewhere. The pencil unrolls the recording's notes below it.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { EyeOff, Loader2, Maximize2, Mic, MicOff, Pause, Pencil, Play, Square, Volume2, VolumeX } from 'lucide-react';
import { LiveAudioVisualizer } from '@/components/LiveAudioVisualizer';
import { NotesEditor } from '@/components/Notes/NotesEditor';
import { useRecordingNotes } from '@/hooks/useMeetingNotes';
import { recordingService } from '@/services/recordingService';
import { applyAppTheme, getSavedAppTheme } from '@/lib/app-theme';
import { barPaths, peelFor, revealFor } from '@/lib/minibar-dock';
import { settleRemaining } from '@/lib/press-settle';
import { ARM_WINDOW_MS, pressTwoStep } from '@/lib/two-step-press';

/** Room the window leaves on each side for the corners of the docked tab. */
const EAR = 10;
const BAR_HEIGHT = 48;
/** What stays on screen of a docked bar tucked away. */
const PEEK = 6;
/** The bar stays out this long after the cursor leaves it. */
const LINGER_MS = 1200;
/** Height of the notes below the bar; the window grows by it (Rust side). */
const NOTES_HEIGHT = 216;
const NOTES_ANIMATION_MS = 180;

/** How long the corners take to peel off the edge, or settle onto it. */
const PEEL_MS = 140;

type NotesState = 'closed' | 'open' | 'closing';

/** `target`, reached over `duration` ms with an ease-out rather than at once. */
function useEased(target: number, duration: number): number {
  const [value, setValue] = useState(target);
  const current = useRef(target);
  useEffect(() => {
    const from = current.current;
    if (from === target) return;
    const start = performance.now();
    let frame = 0;
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / duration);
      current.current = from + (target - from) * (1 - (1 - t) ** 3);
      setValue(current.current);
      if (t < 1) frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [target, duration]);
  return value;
}

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
  // A pause or resume on its way to Rust, held busy for at least
  // PRESS_SETTLE_MS. The ref closes the gap before the disabled button
  // re-renders; the hold outlasts a double click, so it sends one command.
  const [isChangingPause, setIsChangingPause] = useState(false);
  const pauseInFlight = useRef(false);
  // Bumped by every pause/resume, so a poll that left before it cannot bring
  // the old state back after the command has settled.
  const pauseCommands = useRef(0);
  // When the stop button was armed by a first press, or null.
  const [stopArmedAt, setStopArmedAt] = useState<number | null>(null);

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
      const commandsBefore = pauseCommands.current;
      try {
        const state = await recordingService.getRecordingState();
        if (!mounted) return;
        const duration = state.active_duration ?? state.recording_duration;
        if (duration !== null) {
          setElapsed(Math.max(0, Math.floor(duration)));
        }
        if (!pauseInFlight.current && pauseCommands.current === commandsBefore) {
          setIsPaused(state.is_paused);
        }
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
    if (pauseInFlight.current || isStopping) return;
    pauseInFlight.current = true;
    pauseCommands.current += 1;
    setIsChangingPause(true);
    setStopArmedAt(null);
    const startedAt = performance.now();
    try {
      await invoke(isPaused ? 'resume_recording' : 'pause_recording');
      setIsPaused(!isPaused);
    } catch (e) {
      console.error('Compact bar: pause/resume failed', e);
    } finally {
      const wait = settleRemaining(startedAt, performance.now());
      if (wait > 0) await new Promise((resolve) => window.setTimeout(resolve, wait));
      pauseInFlight.current = false;
      pauseCommands.current += 1;
      setIsChangingPause(false);
    }
  }, [isPaused, isStopping]);

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

  // A slip beside the pause button must not end the meeting: the first press
  // only asks, a second one within a few seconds stops.
  const pressStop = useCallback(() => {
    const now = performance.now();
    const outcome = pressTwoStep(stopArmedAt, now);
    if (outcome === 'armed') setStopArmedAt(now);
    else if (outcome === 'confirmed') {
      setStopArmedAt(null);
      void stop();
    }
  }, [stop, stopArmedAt]);

  useEffect(() => {
    if (stopArmedAt === null) return;
    const id = window.setTimeout(() => setStopArmedAt(null), ARM_WINDOW_MS);
    return () => window.clearTimeout(id);
  }, [stopArmedAt]);

  // --- Docking -------------------------------------------------------------
  const [docked, setDocked] = useState(false);
  const [edgeGap, setEdgeGap] = useState(0);
  const [distance, setDistance] = useState(0);
  const [lingering, setLingering] = useState(false);
  const [viewport, setViewport] = useState({ width: 440, height: BAR_HEIGHT });

  useEffect(() => {
    let disposed = false;
    const unlisteners = [
      listen<boolean>('minibar-dock', (event) => setDocked(event.payload)),
      listen<number>('minibar-pointer', (event) => setDistance(event.payload)),
      listen<number>('minibar-edge-gap', (event) => setEdgeGap(event.payload)),
    ];
    invoke<boolean>('minibar_dock_state')
      .then((state) => !disposed && setDocked(state))
      .catch((error) => console.error('Compact bar: failed to read the dock state', error));
    const measure = () => setViewport({ width: window.innerWidth, height: window.innerHeight });
    measure();
    window.addEventListener('resize', measure);
    return () => {
      disposed = true;
      window.removeEventListener('resize', measure);
      unlisteners.forEach((pending) => void pending.then((unlisten) => unlisten()));
    };
  }, []);

  // Leaving the bar does not hide it at once: a glance away is not "done".
  const cursorWasOn = useRef(true);
  const lingerTimer = useRef<number | undefined>(undefined);
  useEffect(() => {
    if (distance <= 0) {
      window.clearTimeout(lingerTimer.current);
      cursorWasOn.current = true;
      setLingering(false);
      return;
    }
    if (!cursorWasOn.current) return;
    cursorWasOn.current = false;
    setLingering(true);
    lingerTimer.current = window.setTimeout(() => setLingering(false), LINGER_MS);
  }, [distance]);
  useEffect(() => () => window.clearTimeout(lingerTimer.current), []);

  // --- Notes ---------------------------------------------------------------
  const recordingNotes = useRecordingNotes(true);
  const [notes, setNotes] = useState<NotesState>('closed');
  const [typing, setTyping] = useState(false);
  const [windowFocused, setWindowFocused] = useState(true);
  const notesRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const focus = () => setWindowFocused(true);
    const blur = () => setWindowFocused(false);
    window.addEventListener('focus', focus);
    window.addEventListener('blur', blur);
    return () => {
      window.removeEventListener('focus', focus);
      window.removeEventListener('blur', blur);
    };
  }, []);

  const toggleNotes = useCallback(async () => {
    if (notes === 'closed') {
      try {
        // The window grows first, so the notes unroll into room already there.
        await invoke('set_minibar_notes_open', { open: true });
        setNotes('open');
      } catch (error) {
        console.error('Compact bar: failed to open the notes', error);
      }
    } else if (notes === 'open') {
      setNotes('closing');
      window.setTimeout(() => {
        invoke('set_minibar_notes_open', { open: false })
          .catch((error) => console.error('Compact bar: failed to close the notes', error))
          .finally(() => setNotes('closed'));
      }, NOTES_ANIMATION_MS);
    }
  }, [notes]);

  /** Unrolled: the caret waits at the end of the text. */
  const focusNotes = () => {
    if (notes !== 'open') return;
    const lines = notesRef.current?.querySelectorAll('textarea');
    lines?.[lines.length - 1]?.focus();
  };

  // --- Motion --------------------------------------------------------------
  const reveal = docked ? revealFor(distance, lingering || (typing && windowFocused)) : 1;
  const previousReveal = useRef(reveal);
  const hiding = reveal < previousReveal.current;
  useEffect(() => {
    previousReveal.current = reveal;
  }, [reveal]);
  const offset = -(1 - reveal) * Math.max(0, viewport.height - PEEK);
  const motion = hiding
    ? 'transform 520ms cubic-bezier(0.4, 0, 0.2, 1), opacity 520ms ease'
    : 'transform 140ms ease-out, opacity 140ms ease-out';
  const peel = useEased(peelFor(docked, edgeGap), PEEL_MS);
  const shape = barPaths(viewport.width, BAR_HEIGHT, EAR, 16, peel);

  const busy = isStopping || isChangingMicMute || isChangingSystemMute;
  const stopArmed = stopArmedAt !== null && !isStopping;
  const statusLabel = isStopping ? tr('stopping') : isPaused ? tr('paused') : tr('recordingLabel');
  const pauseLabel = isPaused ? t('minibarResume') : t('minibarPause');
  const icon =
    'flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] disabled:opacity-40';
  const sources = [
    ['mic', isMicMuted, toggleMicMute, tr('muteMicrophone'), tr('unmuteMicrophone'), Mic, MicOff],
    ['system', isSystemMuted, toggleSystemMute, tr('muteSystemAudio'), tr('unmuteSystemAudio'), Volume2, VolumeX],
  ] as const;

  return (
    <div className="relative h-screen w-screen select-none overflow-hidden">
      <div
        className="absolute inset-x-0 top-0"
        style={{ transform: `translateY(${offset}px)`, opacity: 0.5 + 0.5 * reveal, transition: motion }}
      >
        <div className="relative" style={{ height: BAR_HEIGHT }}>
          {/* Against the edge, a tab whose top corners spread into it; away
              from it, a pill. One shape, eased between the two. */}
          <svg
            className="pointer-events-none absolute left-0 top-0"
            width={viewport.width}
            height={BAR_HEIGHT}
            aria-hidden
          >
            <path d={shape.fill} style={{ fill: 'var(--af-panel)' }} />
            <path d={shape.outline} style={{ fill: 'none', stroke: 'var(--af-border-strong)', strokeWidth: 1 }} />
            <path
              d={shape.top}
              style={{ fill: 'none', stroke: 'var(--af-border-strong)', strokeWidth: 1, opacity: peel }}
            />
          </svg>
          <div
            data-tauri-drag-region
            className="relative flex h-full items-center gap-1.5 text-[var(--af-text)]"
            style={{ paddingLeft: EAR + 14, paddingRight: EAR + 6 }}
          >
            {/* Status and timer. The words are in the tooltip; the mark differs
                in shape as well as colour — a dot while recording, two bars on
                pause, a spinner while stopping — so it reads without colour. */}
            <div
              data-tauri-drag-region
              className="flex shrink-0 items-center gap-2"
              title={statusLabel}
            >
              <span data-tauri-drag-region className="flex h-2.5 w-2.5 items-center justify-center" aria-hidden>
                {isStopping ? (
                  <Loader2 size={10} className="animate-spin text-[var(--af-text-3)]" />
                ) : isPaused ? (
                  <Pause size={11} className="text-orange-400" fill="currentColor" strokeWidth={0} />
                ) : (
                  <span className="h-2.5 w-2.5 animate-pulse rounded-full bg-red-500" />
                )}
              </span>
              <span role="status" className="sr-only">
                {statusLabel}
              </span>
              <span
                data-tauri-drag-region
                className={`text-sm font-medium tabular-nums ${isPaused || isStopping ? 'text-[var(--af-text-3)]' : ''}`}
              >
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
                {/* The bar is full to the last pixel; the armed stop button
                    borrows the levels' room for its word. */}
                <LiveAudioVisualizer
                  active={!isPaused && !isStopping && !muted}
                  source={source}
                  bars={6}
                  className={stopArmed ? '!hidden' : ''}
                />
              </div>
            ))}

            <div data-tauri-drag-region className="flex-1" />

            <button
              type="button"
              onClick={() => void togglePause()}
              disabled={busy || isChangingPause}
              title={pauseLabel}
              aria-label={pauseLabel}
              aria-busy={isChangingPause}
              className={icon}
            >
              {isChangingPause ? (
                <Loader2 size={14} className="animate-spin" />
              ) : isPaused ? (
                <Play size={14} />
              ) : (
                <Pause size={14} />
              )}
            </button>
            <button
              type="button"
              onClick={pressStop}
              disabled={busy || isChangingPause}
              title={stopArmed ? t('minibarStopConfirmTitle') : t('minibarStopTitle')}
              aria-label={stopArmed ? t('minibarStopConfirmTitle') : t('minibarStopTitle')}
              className={
                stopArmed
                  ? 'flex h-7 shrink-0 items-center gap-1 rounded-full bg-red-500 px-2 text-xs font-medium text-white transition-colors hover:bg-red-600 disabled:opacity-40'
                  : `${icon} !text-red-500 hover:!bg-red-500/10`
              }
            >
              <Square size={stopArmed ? 10 : 12} fill="currentColor" />
              {stopArmed && <span>{t('minibarStopConfirm')}</span>}
            </button>

            <span data-tauri-drag-region className="mx-0.5 h-5 w-px shrink-0 bg-[var(--af-border)]" />

            <button
              type="button"
              onClick={() => void toggleNotes()}
              disabled={isStopping || notes === 'closing'}
              title={notes === 'open' ? t('minibarNotesHide') : t('minibarNotesShow')}
              aria-label={notes === 'open' ? t('minibarNotesHide') : t('minibarNotesShow')}
              aria-pressed={notes === 'open'}
              className={`${icon} ${notes === 'open' ? 'bg-[var(--af-accent-soft)] !text-[var(--af-accent)]' : ''}`}
            >
              <Pencil size={13} />
            </button>
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
        </div>

        {notes !== 'closed' && (
          <div
            ref={notesRef}
            className={`overflow-hidden rounded-xl border border-[var(--af-border-strong)] bg-[var(--af-panel)] ${
              notes === 'closing' ? 'minibar-roll-up' : 'minibar-unroll'
            }`}
            style={{ margin: `4px ${EAR + 4}px 0`, height: NOTES_HEIGHT - 8 }}
            onAnimationEnd={focusNotes}
            onFocusCapture={() => setTyping(true)}
            onBlurCapture={() => setTyping(false)}
          >
            <NotesEditor source={recordingNotes} live compact />
          </div>
        )}
      </div>
    </div>
  );
}

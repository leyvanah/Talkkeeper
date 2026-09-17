/**
 * The playhead, shared without re-rendering.
 *
 * The player's position changes every animation frame. Passing it down as a
 * prop would re-render everything that shows it at that rate, and the table
 * view is not cheap to re-render. So the player writes the position here and
 * whatever draws it subscribes and moves its own element directly.
 *
 * Pure on purpose: no React, so it can be tested with plain node.
 */

export type PlayheadListener = (seconds: number, playing: boolean) => void;

export interface Playhead {
  /** Seconds into the recording. */
  time(): number;
  playing(): boolean;
  /** Called by the player. Listeners hear only real changes. */
  update(seconds: number, playing: boolean): void;
  /** Hears the current state at once, then every change. Returns an unsubscribe. */
  subscribe(listener: PlayheadListener): () => void;
}

export function createPlayhead(): Playhead {
  let seconds = 0;
  let isPlaying = false;
  const listeners = new Set<PlayheadListener>();

  return {
    time: () => seconds,
    playing: () => isPlaying,
    update(nextSeconds, nextPlaying) {
      if (nextSeconds === seconds && nextPlaying === isPlaying) return;
      seconds = nextSeconds;
      isPlaying = nextPlaying;
      for (const listener of listeners) listener(seconds, isPlaying);
    },
    subscribe(listener) {
      listeners.add(listener);
      listener(seconds, isPlaying);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

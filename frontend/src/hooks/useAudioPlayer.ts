import { useCallback, useEffect, useRef, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';

/**
 * Plays a stored recording.
 *
 * The file is fetched over the application's own URL scheme (registered in
 * `audio/recording_protocol.rs`) rather than read into the page: the webview
 * asks for the parts it is playing and nothing more. Before, the whole file
 * crossed the IPC boundary as a JSON array of numbers and was decoded into
 * memory in full — for an hour of audio, about 150 MB of text and a 700 MB
 * buffer, before the first sound.
 *
 * `currentTime` is updated per animation frame while playing rather than from
 * `timeupdate`, which fires about four times a second: too coarse to follow a
 * transcript with.
 */

/** Must match `recording_protocol::SCHEME`. */
const RECORDING_SCHEME = 'recording';

export const useAudioPlayer = (audioPath: string | null) => {
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const elementRef = useRef<HTMLAudioElement | null>(null);
  const frameRef = useRef<number | null>(null);

  const stopFollowing = useCallback(() => {
    if (frameRef.current !== null) {
      cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }
  }, []);

  const follow = useCallback(() => {
    stopFollowing();
    const step = () => {
      const element = elementRef.current;
      if (!element) return;
      setCurrentTime(element.currentTime);
      frameRef.current = requestAnimationFrame(step);
    };
    frameRef.current = requestAnimationFrame(step);
  }, [stopFollowing]);

  useEffect(() => {
    setIsPlaying(false);
    setCurrentTime(0);
    setDuration(0);
    setError(null);

    if (!audioPath) {
      elementRef.current = null;
      return;
    }

    const element = new Audio(convertFileSrc(audioPath, RECORDING_SCHEME));
    // Metadata is enough to draw the timeline; the audio itself follows the
    // playhead, so opening a long meeting costs one small request.
    element.preload = 'metadata';
    elementRef.current = element;

    const onLoaded = () => setDuration(Number.isFinite(element.duration) ? element.duration : 0);
    const onPlay = () => {
      setIsPlaying(true);
      follow();
    };
    const onPause = () => {
      setIsPlaying(false);
      stopFollowing();
      setCurrentTime(element.currentTime);
    };
    const onEnded = () => {
      setIsPlaying(false);
      stopFollowing();
      setCurrentTime(0);
    };
    const onError = () => {
      stopFollowing();
      setIsPlaying(false);
      setError(describe(element.error));
    };
    const onSeeked = () => setCurrentTime(element.currentTime);

    element.addEventListener('loadedmetadata', onLoaded);
    element.addEventListener('play', onPlay);
    element.addEventListener('pause', onPause);
    element.addEventListener('ended', onEnded);
    element.addEventListener('error', onError);
    element.addEventListener('seeked', onSeeked);

    return () => {
      stopFollowing();
      element.pause();
      element.removeEventListener('loadedmetadata', onLoaded);
      element.removeEventListener('play', onPlay);
      element.removeEventListener('pause', onPause);
      element.removeEventListener('ended', onEnded);
      element.removeEventListener('error', onError);
      element.removeEventListener('seeked', onSeeked);
      // Let the webview drop the connection rather than keep reading a file
      // the page has moved on from.
      element.removeAttribute('src');
      element.load();
      if (elementRef.current === element) {
        elementRef.current = null;
      }
    };
  }, [audioPath, follow, stopFollowing]);

  const play = useCallback(async () => {
    const element = elementRef.current;
    if (!element) return;
    try {
      await element.play();
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Could not play the recording');
    }
  }, []);

  const pause = useCallback(() => {
    elementRef.current?.pause();
  }, []);

  const seek = useCallback((time: number) => {
    const element = elementRef.current;
    if (!element) return;
    const limit = Number.isFinite(element.duration) ? element.duration : time;
    const target = Math.min(Math.max(time, 0), limit);
    element.currentTime = target;
    setCurrentTime(target);
  }, []);

  return { isPlaying, currentTime, duration, error, play, pause, seek };
};

/** What went wrong, in terms a caller can show or log. */
function describe(mediaError: MediaError | null): string {
  switch (mediaError?.code) {
    case MediaError.MEDIA_ERR_ABORTED:
      return 'Playback was stopped';
    case MediaError.MEDIA_ERR_NETWORK:
      return 'The recording could not be read';
    case MediaError.MEDIA_ERR_DECODE:
      return 'The recording could not be decoded';
    case MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED:
      return 'The recording is missing or of an unsupported format';
    default:
      return 'The recording could not be played';
  }
}

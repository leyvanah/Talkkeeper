/**
 * Rules for the recording settings that are worth more than a component:
 * which switch may not be turned off, which echo option is open, and which
 * recognition model the settings recommend.
 */

/** How the speakers' echo is kept out of the microphone (`echo_mode`). */
export type EchoMode = 'windows' | 'builtin' | 'off';

/**
 * The switch that may not be turned off now. A recording keeps its audio, its
 * live text, or both: with neither, the meeting is saved empty.
 */
export function lockedRecordingSwitch(
  autoSave: boolean,
  liveText: boolean,
): 'autoSave' | 'liveText' | null {
  if (autoSave && !liveText) return 'autoSave';
  if (liveText && !autoSave) return 'liveText';
  return null;
}

/**
 * The echo option shown as chosen. Windows' own cleaning exists only on
 * Windows; elsewhere the backend falls back to the built-in canceller, and
 * the settings say so rather than show a choice that does nothing.
 */
export function shownEchoMode(mode: EchoMode | undefined, isWindows: boolean): EchoMode {
  const chosen = mode ?? 'windows';
  return chosen === 'windows' && !isWindows ? 'builtin' : chosen;
}

/** The model recommended for live text: GigaAM for a Russian interface. */
export function recommendedLiveModel(locale: string): 'gigaam' | 'parakeet' {
  return locale === 'ru' ? 'gigaam' : 'parakeet';
}

/** The model recommended for the pass after recording. */
export function recommendedPostCallModel(locale: string): 'gigaam' | 'whisper' {
  return locale === 'ru' ? 'gigaam' : 'whisper';
}

/**
 * What a toast says after "Identify speakers" or "Transcribe again", from
 * what the pass actually did to the meeting's lines.
 *
 * Both leave alone what a person chose — names, corrected lines — so a pass
 * can finish having changed nothing at all. The toast used to announce the
 * speakers found and the lines labelled either way, and a run with no visible
 * effect read as a broken button.
 */

/** What `diarize_meeting` reports about the lines. */
export interface SpeakerPassResult {
  num_speakers: number;
  /** Lines whose label is different now. */
  changed: number;
  /** Lines left as they were. */
  unchanged: number;
  /** Of the unchanged, lines with a speaker a person chose or named. */
  kept_by_hand: number;
}

/** A message key in `meetingDetails` and the values it takes. */
export interface OutcomeLine {
  key: string;
  values?: Record<string, number>;
}

export interface OutcomeText {
  title: OutcomeLine;
  description: OutcomeLine | null;
}

export function speakerPassText(result: SpeakerPassResult): OutcomeText {
  const { changed, unchanged, kept_by_hand: keptByHand } = result;
  if (changed > 0) {
    return {
      title: { key: 'speakersChanged', values: { count: changed } },
      description:
        unchanged === 0
          ? null
          : keptByHand > 0
            ? { key: 'speakersUnchangedWithNames', values: { count: unchanged, named: keptByHand } }
            : { key: 'speakersUnchangedCount', values: { count: unchanged } },
    };
  }
  if (unchanged > 0) {
    return {
      title: { key: 'speakersNothingChanged' },
      description: {
        key: keptByHand > 0 ? 'speakersAlreadyLabeledNamesKept' : 'speakersAlreadyLabeled',
      },
    };
  }
  // No lines carry a speaker before or after: nothing was told apart.
  return { title: { key: 'noSpeakersDetected' }, description: null };
}

/** What the retranscription reports about the lines. */
export interface RetranscriptionCounts {
  segments_count: number;
  /** Lines a person corrected or wrote, left as they were. */
  kept_count?: number;
}

export function retranscriptionText(result: RetranscriptionCounts): OutcomeText {
  const kept = result.kept_count ?? 0;
  return {
    title: { key: 'retranscribeDone', values: { count: result.segments_count } },
    description: kept > 0 ? { key: 'retranscribeKeptEdited', values: { count: kept } } : null,
  };
}

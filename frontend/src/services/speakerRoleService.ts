/**
 * Speaker roles: which side of the conversation each speaker of a meeting is on.
 *
 * The host holds the conversation (the local microphone, "You"); the client is
 * the other side. The backend works this out from the labels, so a call needs
 * no setup. An assignment overrides it per speaker per meeting — for a
 * recording made in one room, where both voices share a microphone.
 */

import { invoke } from '@tauri-apps/api/core';

export type SpeakerRole = 'host' | 'client';

/** Where a speaker's lines go. `both` is a line two people spoke at once. */
export type SpeakerSideKind = SpeakerRole | 'both';

export interface SpeakerSide {
  speaker: string;
  side: SpeakerSideKind;
  /** True when a person chose the side rather than the label implying it. */
  assigned: boolean;
}

export function getSpeakerSides(meetingId: string): Promise<SpeakerSide[]> {
  return invoke<SpeakerSide[]>('api_get_speaker_sides', { meetingId });
}

/** Put a speaker on a side, or pass `null` to let the label decide again. */
export function assignSpeakerRole(
  meetingId: string,
  speaker: string,
  role: SpeakerRole | null,
): Promise<void> {
  return invoke<void>('api_assign_speaker_role', { meetingId, speaker, role });
}

/** The local microphone's label — the same test the backend makes. */
function isLocalSpeaker(label: string): boolean {
  return /^you\b/i.test(label) || /\(\s*you\s*\)$/i.test(label);
}

/**
 * Look a label up in a meeting's sides. A label the backend has not reported
 * yet (a live line not saved) gets the side its label implies.
 */
export function sideOf(sides: SpeakerSide[], speaker?: string | null): SpeakerSideKind {
  const label = speaker?.trim() ?? '';
  const known = sides.find((entry) => entry.speaker === label);
  if (known) return known.side;
  const parts = label.split(' + ').map((part) => part.trim()).filter(Boolean);
  const local = parts.some(isLocalSpeaker);
  const other = parts.some((part) => !isLocalSpeaker(part));
  if (local && other) return 'both';
  return local ? 'host' : 'client';
}

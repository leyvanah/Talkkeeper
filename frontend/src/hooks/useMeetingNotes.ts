/**
 * The owner's notes, from either of the two places they live.
 *
 * While a recording runs they are kept by the backend in the recording's
 * journal (sealed, crash-safe); once the meeting is saved they are in the
 * database with it. Both hooks give the notes editor the same shape: the
 * notes are lines of one text, and each call answers with the line as stored.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslations } from 'next-intl';
import { toast } from 'sonner';

export interface MeetingNote {
  id: string;
  /** Seconds into the recording, for a note written while it ran. */
  at?: number;
  text: string;
  writtenAt: string;
}

export interface AddNoteOptions {
  /** Where the recording was when typing the line began. */
  startedAt?: number;
  /** The line it follows; `null` puts it first. */
  after: string | null;
}

export interface NotesSource {
  /** Changes when the notes belong to something else: the editor starts over. */
  identity: string;
  notes: MeetingNote[];
  loading: boolean;
  add: (text: string, options: AddNoteOptions) => Promise<MeetingNote>;
  edit: (id: string, text: string) => Promise<MeetingNote>;
  remove: (id: string) => Promise<void>;
}

/** `note` placed right after `after`, or first; as the backend does it. */
function insertAfter(notes: MeetingNote[], note: MeetingNote, after: string | null): MeetingNote[] {
  const found = after === null ? -1 : notes.findIndex((existing) => existing.id === after);
  const index = after === null ? 0 : found === -1 ? notes.length : found + 1;
  return [...notes.slice(0, index), note, ...notes.slice(index)];
}

/** Notes of the recording now running. */
export function useRecordingNotes(isRecording: boolean): NotesSource {
  const [notes, setNotes] = useState<MeetingNote[]>([]);
  const [loading, setLoading] = useState(false);
  // Changes only once the notes are in, so the editor starts from them.
  const [identity, setIdentity] = useState('idle');

  // A window opened or reloaded mid-recording picks up what was written.
  useEffect(() => {
    if (!isRecording) {
      setNotes([]);
      setIdentity('idle');
      return;
    }
    let cancelled = false;
    setLoading(true);
    invoke<MeetingNote[]>('recording_notes')
      .then((stored) => !cancelled && setNotes(stored))
      .catch((error) => console.error('[Notes] Could not read the recording notes:', error))
      .finally(() => {
        if (cancelled) return;
        setLoading(false);
        setIdentity('recording');
      });
    return () => {
      cancelled = true;
    };
  }, [isRecording]);

  const add = useCallback(async (text: string, { startedAt, after }: AddNoteOptions) => {
    const note = await invoke<MeetingNote>('recording_note_add', { text, startedAt: startedAt ?? null, after });
    setNotes((previous) => insertAfter(previous, note, after));
    return note;
  }, []);

  const edit = useCallback(async (id: string, text: string) => {
    const note = await invoke<MeetingNote>('recording_note_edit', { id, text });
    setNotes((previous) => previous.map((existing) => (existing.id === id ? note : existing)));
    return note;
  }, []);

  const remove = useCallback(async (id: string) => {
    await invoke('recording_note_remove', { id });
    setNotes((previous) => previous.filter((existing) => existing.id !== id));
  }, []);

  return { identity, notes, loading, add, edit, remove };
}

/** Notes of a saved meeting. */
export function useSavedMeetingNotes(meetingId: string | undefined): NotesSource {
  const t = useTranslations('notes');
  const [notes, setNotes] = useState<MeetingNote[]>([]);
  const [loading, setLoading] = useState(true);
  // Changes only once the meeting's notes are in, so the editor starts from them.
  const [identity, setIdentity] = useState('');

  useEffect(() => {
    setNotes([]);
    if (!meetingId) return;
    let cancelled = false;
    setLoading(true);
    invoke<MeetingNote[]>('api_get_meeting_notes', { meetingId })
      .then((stored) => !cancelled && setNotes(stored))
      .catch((failure) => {
        console.error('[Notes] Could not read the meeting notes:', failure);
        if (!cancelled) toast.error(t('loadFailed'), { description: String(failure) });
      })
      .finally(() => {
        if (cancelled) return;
        setLoading(false);
        setIdentity(`meeting:${meetingId}`);
      });
    return () => {
      cancelled = true;
    };
    // `t` changes with the language only; reading the notes again for that is not needed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [meetingId]);

  const add = useCallback(
    async (text: string, { after }: AddNoteOptions) => {
      const note = await invoke<MeetingNote>('api_add_meeting_note', { meetingId, text, after });
      setNotes((previous) => insertAfter(previous, note, after));
      return note;
    },
    [meetingId],
  );

  const edit = useCallback(
    async (id: string, text: string) => {
      const note = await invoke<MeetingNote>('api_edit_meeting_note', { meetingId, id, text });
      setNotes((previous) => previous.map((existing) => (existing.id === id ? note : existing)));
      return note;
    },
    [meetingId],
  );

  const remove = useCallback(
    async (id: string) => {
      await invoke('api_remove_meeting_note', { meetingId, id });
      setNotes((previous) => previous.filter((existing) => existing.id !== id));
    },
    [meetingId],
  );

  return { identity, notes, loading, add, edit, remove };
}

/** Where the recording now running is, in seconds of recorded sound. */
export async function recordingPosition(): Promise<number | undefined> {
  try {
    const state = await invoke<{ active_duration?: number | null }>('get_recording_state');
    return typeof state.active_duration === 'number' ? state.active_duration : undefined;
  } catch {
    return undefined;
  }
}

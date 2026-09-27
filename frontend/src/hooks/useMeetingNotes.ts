/**
 * The owner's notes, from either of the two places they live.
 *
 * While a recording runs they are kept by the backend in the recording's
 * journal (sealed, crash-safe); once the meeting is saved they are in the
 * database with it. Both hooks give the notes panel the same shape.
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

export interface NotesSource {
  notes: MeetingNote[];
  loading: boolean;
  /** `startedAt`: where the recording was when typing began. */
  add: (text: string, startedAt?: number) => Promise<void>;
  edit: (id: string, text: string) => Promise<void>;
  remove: (id: string) => Promise<void>;
}

/** Notes of the recording now running. */
export function useRecordingNotes(isRecording: boolean): NotesSource {
  const [notes, setNotes] = useState<MeetingNote[]>([]);
  const [loading, setLoading] = useState(false);

  // A window opened or reloaded mid-recording picks up what was written.
  useEffect(() => {
    if (!isRecording) {
      setNotes([]);
      return;
    }
    let cancelled = false;
    setLoading(true);
    invoke<MeetingNote[]>('recording_notes')
      .then((stored) => !cancelled && setNotes(stored))
      .catch((error) => console.error('[Notes] Could not read the recording notes:', error))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [isRecording]);

  const add = useCallback(async (text: string, startedAt?: number) => {
    const note = await invoke<MeetingNote>('recording_note_add', { text, startedAt: startedAt ?? null });
    setNotes((previous) => [...previous, note]);
  }, []);

  const edit = useCallback(async (id: string, text: string) => {
    const note = await invoke<MeetingNote>('recording_note_edit', { id, text });
    setNotes((previous) => previous.map((existing) => (existing.id === id ? note : existing)));
  }, []);

  const remove = useCallback(async (id: string) => {
    await invoke('recording_note_remove', { id });
    setNotes((previous) => previous.filter((existing) => existing.id !== id));
  }, []);

  return { notes, loading, add, edit, remove };
}

/** Notes of a saved meeting. */
export function useSavedMeetingNotes(meetingId: string | undefined): NotesSource {
  const t = useTranslations('notes');
  const [notes, setNotes] = useState<MeetingNote[]>([]);
  const [loading, setLoading] = useState(false);

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
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
    // `t` changes with the language only; reading the notes again for that is not needed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [meetingId]);

  const add = useCallback(
    async (text: string) => {
      if (!meetingId) return;
      setNotes(await invoke<MeetingNote[]>('api_add_meeting_note', { meetingId, text }));
    },
    [meetingId],
  );

  const edit = useCallback(
    async (id: string, text: string) => {
      if (!meetingId) return;
      setNotes(await invoke<MeetingNote[]>('api_edit_meeting_note', { meetingId, id, text }));
    },
    [meetingId],
  );

  const remove = useCallback(
    async (id: string) => {
      if (!meetingId) return;
      setNotes(await invoke<MeetingNote[]>('api_remove_meeting_note', { meetingId, id }));
    },
    [meetingId],
  );

  return { notes, loading, add, edit, remove };
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

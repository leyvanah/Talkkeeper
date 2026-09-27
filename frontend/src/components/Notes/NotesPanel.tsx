'use client';

/**
 * The owner's own notes: the list, and a box to write the next one.
 *
 * Shown in two places. Beside a recording in progress (`live`), where each
 * note is placed at the moment of the recording the owner began typing it —
 * a note is about what was just said, and typing takes a while. And on a
 * saved meeting's Notes tab, where a note's moment is a link to that place in
 * the recording, and a note written afterwards has none.
 */

import { useEffect, useRef, useState } from 'react';
import { useTranslations } from 'next-intl';
import { NotebookPen, Pencil, Send, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import type { MeetingNote, NotesSource } from '@/hooks/useMeetingNotes';
import { recordingPosition } from '@/hooks/useMeetingNotes';

/** `MM:SS`, or `H:MM:SS` past the hour — as the transcript shows time. */
export function noteClock(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  const pad = (value: number) => String(value).padStart(2, '0');
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(secs)}` : `${pad(minutes)}:${pad(secs)}`;
}

const describe = (error: unknown) => (typeof error === 'string' ? error : String(error));

interface NotesPanelProps {
  source: NotesSource;
  /** A recording is running: new notes get their moment in it. */
  live: boolean;
  /** Plays the recording from a note's moment. */
  onSeek?: (seconds: number) => void;
}

export function NotesPanel({ source, live, onSeek }: NotesPanelProps) {
  const t = useTranslations('notes');
  const { notes, loading, add, edit, remove } = source;
  const [draft, setDraft] = useState('');
  const [saving, setSaving] = useState(false);
  // Where the recording was when the owner began typing this draft.
  const startedAt = useRef<number | undefined>(undefined);
  const listRef = useRef<HTMLDivElement>(null);
  const lastCount = useRef(notes.length);

  // A new note scrolls into view; an edit or a delete leaves the list where it is.
  useEffect(() => {
    if (notes.length > lastCount.current) {
      listRef.current?.scrollTo({ top: listRef.current.scrollHeight, behavior: 'smooth' });
    }
    lastCount.current = notes.length;
  }, [notes.length]);

  const changeDraft = (value: string) => {
    if (live && !draft.trim() && value.trim()) {
      void recordingPosition().then((position) => {
        startedAt.current = position;
      });
    }
    if (!value.trim()) startedAt.current = undefined;
    setDraft(value);
  };

  const submit = async () => {
    const text = draft.trim();
    if (!text || saving) return;
    setSaving(true);
    try {
      await add(text, live ? startedAt.current : undefined);
      setDraft('');
      startedAt.current = undefined;
    } catch (error) {
      console.error('[Notes] Could not add a note:', error);
      toast.error(t('saveFailed'), { description: describe(error) });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
        {notes.length === 0 ? (
          !loading && (
            <div className="flex h-full flex-col items-center justify-center gap-2 px-4 text-center">
              <NotebookPen size={20} className="text-[var(--af-text-3)]" />
              <p className="max-w-xs text-sm text-[var(--af-text-3)]">{live ? t('emptyLive') : t('empty')}</p>
            </div>
          )
        ) : (
          <ul className="space-y-1">
            {notes.map((note) => (
              <NoteItem key={note.id} note={note} onEdit={edit} onRemove={remove} onSeek={onSeek} />
            ))}
          </ul>
        )}
      </div>

      <div className="px-4 pb-4">
        <div className="flex items-end gap-2 rounded-xl border border-[var(--af-border)] bg-[var(--af-panel)] py-1.5 pl-4 pr-1.5 focus-within:border-[var(--af-border-strong)]">
          <textarea
            value={draft}
            onChange={(event) => changeDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                void submit();
              }
            }}
            rows={Math.min(6, Math.max(1, draft.split('\n').length))}
            placeholder={live ? t('placeholderLive') : t('placeholder')}
            title={t('newLineHint')}
            aria-label={t('add')}
            className="af-bare flex-1 resize-none border-0 bg-transparent py-1 text-sm text-[var(--af-text)] placeholder:text-[var(--af-text-3)] focus:outline-none focus:ring-0"
          />
          <button
            type="button"
            onClick={() => void submit()}
            disabled={saving || !draft.trim()}
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] disabled:opacity-40 disabled:hover:bg-transparent"
            title={t('add')}
            aria-label={t('add')}
          >
            <Send size={16} />
          </button>
        </div>
        {notes.length > 0 && <p className="mt-1.5 px-1 text-xs text-[var(--af-text-3)]">{t('summaryHint')}</p>}
      </div>
    </div>
  );
}

interface NoteItemProps {
  note: MeetingNote;
  onEdit: (id: string, text: string) => Promise<void>;
  onRemove: (id: string) => Promise<void>;
  onSeek?: (seconds: number) => void;
}

function NoteItem({ note, onEdit, onRemove, onSeek }: NoteItemProps) {
  const t = useTranslations('notes');
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(note.text);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);

  // A second click deletes; the question goes away by itself.
  useEffect(() => {
    if (!confirming) return;
    const timer = setTimeout(() => setConfirming(false), 3000);
    return () => clearTimeout(timer);
  }, [confirming]);

  const save = async () => {
    const trimmed = text.trim();
    if (!trimmed || busy) return;
    if (trimmed === note.text) {
      setEditing(false);
      return;
    }
    setBusy(true);
    try {
      await onEdit(note.id, trimmed);
      setEditing(false);
    } catch (error) {
      console.error('[Notes] Could not edit a note:', error);
      toast.error(t('saveFailed'), { description: describe(error) });
    } finally {
      setBusy(false);
    }
  };

  const removeNote = async () => {
    if (!confirming) {
      setConfirming(true);
      return;
    }
    setBusy(true);
    try {
      await onRemove(note.id);
    } catch (error) {
      console.error('[Notes] Could not delete a note:', error);
      toast.error(t('removeFailed'), { description: describe(error) });
      setBusy(false);
    }
  };

  const moment =
    note.at === undefined || note.at === null ? (
      <span className="text-xs text-[var(--af-text-3)]">{t('afterRecording')}</span>
    ) : onSeek ? (
      <button
        type="button"
        onClick={() => onSeek(note.at!)}
        className="rounded px-1 font-mono text-xs tabular-nums text-[var(--af-accent)] hover:bg-[var(--af-hover)]"
        title={t('seekTo', { time: noteClock(note.at) })}
      >
        {noteClock(note.at)}
      </button>
    ) : (
      <span className="px-1 font-mono text-xs tabular-nums text-[var(--af-text-3)]">{noteClock(note.at)}</span>
    );

  return (
    <li className="group rounded-lg px-2 py-1.5 hover:bg-[var(--af-hover)]">
      <div className="flex items-center gap-2">
        {moment}
        {!editing && (
          <div className="ml-auto flex items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
            <button
              type="button"
              onClick={() => {
                setText(note.text);
                setEditing(true);
              }}
              className="rounded p-1 text-[var(--af-text-3)] hover:text-[var(--af-text)]"
              title={t('edit')}
              aria-label={t('edit')}
            >
              <Pencil size={13} />
            </button>
            <button
              type="button"
              onClick={() => void removeNote()}
              disabled={busy}
              className={`rounded p-1 text-xs ${
                confirming ? 'font-medium text-red-500' : 'text-[var(--af-text-3)] hover:text-[var(--af-text)]'
              }`}
              title={t('remove')}
              aria-label={confirming ? t('confirmRemove') : t('remove')}
            >
              {confirming ? t('confirmRemove') : <Trash2 size={13} />}
            </button>
          </div>
        )}
      </div>
      {editing ? (
        <div className="mt-1 space-y-1.5">
          <textarea
            value={text}
            autoFocus
            onChange={(event) => setText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                void save();
              }
              if (event.key === 'Escape') setEditing(false);
            }}
            rows={Math.min(8, Math.max(2, text.split('\n').length))}
            className="w-full resize-none rounded-md border border-[var(--af-border)] bg-[var(--af-panel)] px-2 py-1 text-sm text-[var(--af-text)] focus:border-[var(--af-border-strong)] focus:outline-none"
          />
          <div className="flex justify-end gap-1">
            <button
              type="button"
              onClick={() => setEditing(false)}
              className="rounded-md px-2 py-1 text-xs text-[var(--af-text-2)] hover:bg-[var(--af-panel-2)]"
            >
              {t('cancel')}
            </button>
            <button
              type="button"
              onClick={() => void save()}
              disabled={busy || !text.trim()}
              className="rounded-md bg-[var(--af-text)] px-2 py-1 text-xs text-[var(--af-bg)] disabled:opacity-40"
            >
              {t('save')}
            </button>
          </div>
        </div>
      ) : (
        <p className="mt-0.5 whitespace-pre-wrap break-words px-1 text-sm text-[var(--af-text)]">{note.text}</p>
      )}
    </li>
  );
}

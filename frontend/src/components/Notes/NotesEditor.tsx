'use client';

/**
 * The owner's own notes, written as one text: each line is a note, and the
 * moment of the recording it belongs to stands in the margin on its left.
 *
 * Shown in two places. Beside a recording in progress (`live`), where a line
 * gets the moment the owner began typing it — a note is about what was just
 * said, and typing takes a while. And on a saved meeting's Notes tab, where a
 * moment is a link to that place in the recording, and a line written
 * afterwards has none.
 *
 * Editing is plain text, the way a notes app does it: Enter starts a new line
 * (splitting the line at the caret), Shift+Enter breaks within one, Backspace
 * at the start of a line joins it to the one above and Delete at its end
 * joins the next, arrows move between lines. Each line saves itself shortly
 * after typing stops and when the caret leaves it; a line emptied is removed.
 */

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { useTranslations } from 'next-intl';
import type { MeetingNote, NotesSource } from '@/hooks/useMeetingNotes';
import { recordingPosition } from '@/hooks/useMeetingNotes';
import { toastFailure } from '@/lib/failure';

/** `MM:SS`, or `H:MM:SS` past the hour — as the transcript shows time. */
export function noteClock(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  const pad = (value: number) => String(value).padStart(2, '0');
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(secs)}` : `${pad(minutes)}:${pad(secs)}`;
}

interface NotesEditorProps {
  source: NotesSource;
  /** A recording is running: new lines get their moment in it. */
  live: boolean;
  /** Plays the recording from a line's moment. */
  onSeek?: (seconds: number) => void;
  /** Tight room, as in the compact recording bar: no hints under the text. */
  compact?: boolean;
}

/** The editor starts over whenever the notes belong to something else. */
export function NotesEditor(props: NotesEditorProps) {
  return <EditorBody key={props.source.identity} {...props} />;
}

/** A line of the text; `id` once the backend holds it. */
interface Line {
  key: string;
  id?: string;
  at?: number;
  text: string;
  /** The text as last stored. */
  saved: string;
}

/** Typing pauses this long before a line is saved. */
const SAVE_DELAY = 600;

let lineCounter = 0;
const newKey = () => `line-${lineCounter++}`;

/** There is always an empty line at the end to write the next note in. */
function withDraft(lines: Line[]): Line[] {
  const last = lines[lines.length - 1];
  return last && !last.id && last.text === '' ? lines : [...lines, { key: newKey(), text: '', saved: '' }];
}

const fromNote = (note: MeetingNote): Line => ({ key: note.id, id: note.id, at: note.at, text: note.text, saved: note.text });

// Grows with its text; Chromium sizes the field to its content.
const AUTO_HEIGHT = { fieldSizing: 'content' } as CSSProperties;

function EditorBody({ source, live, onSeek, compact = false }: NotesEditorProps) {
  const t = useTranslations('notes');
  const [lines, setLinesState] = useState<Line[]>(() => withDraft(source.notes.map(fromNote)));
  // The saves run after renders; they read the text as it is by then.
  const linesRef = useRef(lines);
  const setLines = useCallback((change: (current: Line[]) => Line[]) => {
    const next = withDraft(change(linesRef.current));
    linesRef.current = next;
    setLinesState(next);
  }, []);
  const patch = useCallback(
    (key: string, changes: Partial<Line>) =>
      setLines((current) => current.map((line) => (line.key === key ? { ...line, ...changes } : line))),
    [setLines],
  );

  // One save at a time, in order: a line is added before it can be edited.
  const queue = useRef<Promise<void>>(Promise.resolve());
  const enqueue = useCallback(
    (work: () => Promise<void>) => {
      queue.current = queue.current.then(work).catch((error) => {
        toastFailure(t('saveFailed'), 'note-save', error);
      });
    },
    [t],
  );

  /** Brings the backend up to the line as it now stands. */
  const persist = useCallback(
    async (key: string) => {
      const current = linesRef.current;
      const index = current.findIndex((line) => line.key === key);
      const line = current[index];
      if (!line) return;
      const text = line.text.trim();
      if (!text) {
        if (line.id) {
          await source.remove(line.id);
          patch(key, { id: undefined, saved: '' });
        }
        return;
      }
      if (!line.id) {
        const after = current.slice(0, index).reverse().find((previous) => previous.id)?.id ?? null;
        const note = await source.add(text, { startedAt: line.at, after });
        // Joined to another line while it was being added: it is not a note any more.
        if (!linesRef.current.some((existing) => existing.key === key)) {
          await source.remove(note.id);
          return;
        }
        patch(key, { id: note.id, at: note.at, saved: note.text });
        return;
      }
      if (text !== line.saved) {
        const note = await source.edit(line.id, text);
        patch(key, { saved: note.text });
      }
    },
    [patch, source],
  );

  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const cancelTimer = (key: string) => {
    const timer = timers.current.get(key);
    if (timer) clearTimeout(timer);
    timers.current.delete(key);
  };
  const schedule = (key: string) => {
    cancelTimer(key);
    timers.current.set(
      key,
      setTimeout(() => {
        timers.current.delete(key);
        enqueue(() => persist(key));
      }, SAVE_DELAY),
    );
  };
  const flush = (key: string) => {
    cancelTimer(key);
    enqueue(() => persist(key));
  };
  /** A line leaves the text; the note it was goes with it. */
  const dropLine = (key: string) => {
    cancelTimer(key);
    const id = linesRef.current.find((line) => line.key === key)?.id;
    setLines((current) => current.filter((line) => line.key !== key));
    if (id) enqueue(() => source.remove(id));
  };

  // Whatever is still waiting is saved when the editor goes away — a recording
  // stopped, a meeting left.
  useEffect(() => {
    const pending = timers.current;
    return () => {
      pending.forEach((timer, key) => {
        clearTimeout(timer);
        enqueue(() => persist(key));
      });
      pending.clear();
    };
    // Only on the way out; the functions it calls read refs, not render state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** A new line during a recording belongs to where the recording is. */
  const stamp = (key: string) => {
    if (!live) return;
    void recordingPosition().then((at) => {
      const line = linesRef.current.find((existing) => existing.key === key);
      if (at !== undefined && line && line.at === undefined && !line.id) patch(key, { at });
    });
  };

  // Focus asked for by a change of the lines is given once they are drawn.
  const inputs = useRef(new Map<string, HTMLTextAreaElement>());
  const focusRequest = useRef<{ key: string; caret: number } | null>(null);
  const focusLine = (key: string, caret: number) => {
    const input = inputs.current.get(key);
    if (!input) {
      focusRequest.current = { key, caret };
      return;
    }
    input.focus();
    const at = Math.min(caret, input.value.length);
    input.setSelectionRange(at, at);
  };
  useLayoutEffect(() => {
    const request = focusRequest.current;
    if (!request) return;
    focusRequest.current = null;
    focusLine(request.key, request.caret);
  });

  const change = (key: string, text: string) => {
    const line = linesRef.current.find((existing) => existing.key === key);
    if (!line) return;
    patch(key, { text });
    if (!line.id && line.at === undefined && !line.text.trim() && text.trim()) stamp(key);
    schedule(key);
  };

  const keyDown = (event: KeyboardEvent<HTMLTextAreaElement>, key: string) => {
    if (event.nativeEvent.isComposing) return;
    const current = linesRef.current;
    const index = current.findIndex((line) => line.key === key);
    const line = current[index];
    if (!line) return;
    const input = event.currentTarget;
    const { selectionStart: start, selectionEnd: end, value } = input;
    const collapsed = start === end;
    const previous = current[index - 1];
    const next = current[index + 1];

    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      if (!value.trim()) return;
      // At the very start: an empty line opens above, the caret stays with the text.
      if (collapsed && start === 0) {
        setLines((lines) => {
          const at = lines.findIndex((existing) => existing.key === key);
          return [...lines.slice(0, at), { key: newKey(), text: '', saved: '' }, ...lines.slice(at)];
        });
        focusRequest.current = { key, caret: 0 };
        return;
      }
      const before = value.slice(0, start);
      const rest = value.slice(end);
      // At the end, with an empty line already below: go there.
      if (!rest && next && !next.id && !next.text) {
        flush(key);
        focusLine(next.key, 0);
        return;
      }
      const created: Line = { key: newKey(), text: rest, saved: '' };
      setLines((lines) => {
        const at = lines.findIndex((existing) => existing.key === key);
        const copy = [...lines];
        copy[at] = { ...copy[at], text: before };
        copy.splice(at + 1, 0, created);
        return copy;
      });
      flush(key);
      if (rest.trim()) {
        stamp(created.key);
        schedule(created.key);
      }
      focusRequest.current = { key: created.key, caret: 0 };
      return;
    }

    if (event.key === 'Backspace' && collapsed && start === 0 && previous) {
      event.preventDefault();
      const caret = previous.text.length;
      setLines((lines) =>
        lines
          .map((existing) => (existing.key === previous.key ? { ...existing, text: existing.text + value } : existing))
          .filter((existing) => existing.key !== key),
      );
      cancelTimer(key);
      if (line.id) {
        const id = line.id;
        enqueue(() => source.remove(id));
      }
      schedule(previous.key);
      focusRequest.current = { key: previous.key, caret };
      return;
    }

    if (event.key === 'Delete' && collapsed && start === value.length && next && (next.id || next.text)) {
      event.preventDefault();
      setLines((lines) =>
        lines
          .map((existing) => (existing.key === key ? { ...existing, text: existing.text + next.text } : existing))
          .filter((existing) => existing.key !== next.key),
      );
      cancelTimer(next.key);
      if (next.id) {
        const id = next.id;
        enqueue(() => source.remove(id));
      }
      schedule(key);
      focusRequest.current = { key, caret: start };
      return;
    }

    // Between lines the caret moves as through one text.
    const lineHeight = parseFloat(getComputedStyle(input).lineHeight) || 24;
    const oneRow = input.scrollHeight <= lineHeight * 1.6;
    if (event.key === 'ArrowUp' && collapsed && previous && !value.slice(0, start).includes('\n') && (oneRow || start === 0)) {
      event.preventDefault();
      focusLine(previous.key, oneRow ? start : previous.text.length);
      return;
    }
    if (event.key === 'ArrowDown' && collapsed && next && !value.slice(end).includes('\n') && (oneRow || end === value.length)) {
      event.preventDefault();
      focusLine(next.key, oneRow ? start : 0);
      return;
    }
    if (event.key === 'ArrowLeft' && collapsed && start === 0 && previous) {
      event.preventDefault();
      focusLine(previous.key, previous.text.length);
      return;
    }
    if (event.key === 'ArrowRight' && collapsed && start === value.length && next) {
      event.preventDefault();
      focusLine(next.key, 0);
      return;
    }
    if (event.key === 'Escape') input.blur();
  };

  const blur = (key: string) => {
    const current = linesRef.current;
    const index = current.findIndex((line) => line.key === key);
    const line = current[index];
    if (!line) return;
    // An emptied line in the middle of the text goes; the one at the end stays to write in.
    if (!line.text.trim() && index < current.length - 1) dropLine(key);
    else flush(key);
  };

  const onlyDraft = lines.length === 1 && !lines[0].text;
  const hasNotes = lines.some((line) => line.id);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div
        className={`min-h-0 flex-1 cursor-text overflow-y-auto px-3 ${compact ? 'py-2' : 'py-4'}`}
        onMouseDown={(event) => {
          // A click below the text writes at its end.
          if (event.target !== event.currentTarget) return;
          event.preventDefault();
          const last = linesRef.current[linesRef.current.length - 1];
          focusLine(last.key, last.text.length);
        }}
      >
        {lines.map((line) => (
          <div key={line.key} className="flex items-start gap-2">
            <div className="w-12 shrink-0 select-none pt-px text-right font-mono text-[11px] leading-6 tabular-nums">
              {line.at !== undefined &&
                (onSeek ? (
                  <button
                    type="button"
                    tabIndex={-1}
                    onClick={() => onSeek(line.at!)}
                    className="rounded px-0.5 text-[var(--af-accent)] hover:bg-[var(--af-hover)]"
                    title={t('seekTo', { time: noteClock(line.at) })}
                  >
                    {noteClock(line.at)}
                  </button>
                ) : (
                  <span className="text-[var(--af-text-3)]">{noteClock(line.at)}</span>
                ))}
            </div>
            <textarea
              ref={(element) => {
                if (element) inputs.current.set(line.key, element);
                else inputs.current.delete(line.key);
              }}
              value={line.text}
              rows={1}
              spellCheck
              onChange={(event) => change(line.key, event.target.value)}
              onKeyDown={(event) => keyDown(event, line.key)}
              onFocus={(event) => event.currentTarget.scrollIntoView({ block: 'nearest' })}
              onBlur={() => blur(line.key)}
              placeholder={onlyDraft ? t(live ? 'placeholderLive' : 'placeholder') : undefined}
              aria-label={t('title')}
              style={AUTO_HEIGHT}
              className="af-bare block min-h-6 w-full min-w-0 flex-1 resize-none overflow-hidden border-0 bg-transparent p-0 text-sm leading-6 text-[var(--af-text)] placeholder:text-[var(--af-text-3)] focus:outline-none focus:ring-0"
            />
          </div>
        ))}
        {onlyDraft && !source.loading && !compact && (
          <p className="mt-3 pl-14 pr-2 text-xs leading-5 text-[var(--af-text-3)]">{live ? t('emptyLive') : t('empty')}</p>
        )}
      </div>
      {!compact && (
        <p className="shrink-0 px-4 pb-3 pt-1 text-xs text-[var(--af-text-3)]">
          {hasNotes ? t('summaryHint') : t('newLineHint')}
        </p>
      )}
    </div>
  );
}

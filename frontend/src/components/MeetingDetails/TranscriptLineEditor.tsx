"use client";

/**
 * Correcting one line of a saved transcript, in place.
 *
 * Enter saves, Escape cancels, Shift+Enter is a new line for the rare line
 * that wants one. Removing asks first: a removed line does not come back, not
 * even when the recording is recognized again — which is the point.
 *
 * The line can also be cut in two where the cursor stands — for the turn
 * where the recognizer ran two voices together — or joined with the line
 * after it. Each line keeps its own stretch of the recording, so playback
 * still finds it. Who said a line is changed on its speaker's name, not here.
 */

import { useEffect, useRef, useState } from 'react';
import { useTranslations } from 'next-intl';
import { Check, Combine, Scissors, Trash2, X } from 'lucide-react';

export interface LineEditActions {
  /** Save new text for the line. Rejects to keep the editor open. */
  onSave: (text: string) => Promise<void>;
  /** Remove the line. Rejects to keep the editor open. */
  onRemove: () => Promise<void>;
  /** Cut the line in two. Rejects to keep the editor open. */
  onSplit?: (first: string, second: string) => Promise<void>;
  /** Join the line with the one after it. Absent for the last line. */
  onMergeNext?: () => Promise<void>;
}

export function TranscriptLineEditor({
  initialText,
  onSave,
  onRemove,
  onSplit,
  onMergeNext,
  onClose,
}: LineEditActions & {
  initialText: string;
  onClose: () => void;
}) {
  const t = useTranslations('meetingDetails');
  const [text, setText] = useState(initialText);
  // Where the cursor stands, which is where a split cuts.
  const [cursor, setCursor] = useState(initialText.length);
  const [busy, setBusy] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const area = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const element = area.current;
    if (!element) return;
    element.focus();
    element.setSelectionRange(element.value.length, element.value.length);
  }, []);

  const trackCursor = () => {
    const element = area.current;
    if (element) setCursor(element.selectionStart);
  };

  // Grow with the text rather than scroll inside a box.
  useEffect(() => {
    const element = area.current;
    if (!element) return;
    element.style.height = 'auto';
    element.style.height = `${element.scrollHeight}px`;
  }, [text]);

  const trimmed = text.trim();
  const unchanged = trimmed === initialText.trim();
  const first = text.slice(0, cursor).trim();
  const second = text.slice(cursor).trim();
  const canSplit = !!onSplit && !!first && !!second;

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    try {
      await action();
      onClose();
    } catch {
      // The caller has said why; the editor stays open with the text intact.
    } finally {
      setBusy(false);
    }
  };

  const save = () => {
    if (busy) return;
    if (!trimmed) {
      setConfirmRemove(true);
      return;
    }
    if (unchanged) {
      onClose();
      return;
    }
    void run(() => onSave(trimmed));
  };

  const split = () => {
    if (busy || !canSplit) return;
    void run(() => onSplit!(first, second));
  };

  const mergeNext = () => {
    if (busy || !onMergeNext) return;
    // Text typed here first, so the join carries it.
    void run(async () => {
      if (!unchanged && trimmed) await onSave(trimmed);
      await onMergeNext();
    });
  };

  const quiet =
    'inline-flex items-center gap-1 rounded-md px-2 py-1 text-[var(--af-text-2)] hover:bg-[var(--af-panel-2)] disabled:opacity-40';

  return (
    <div
      className="rounded-lg border border-[var(--af-accent)] bg-[var(--af-panel)] p-2 shadow-lg"
      onClick={(event) => event.stopPropagation()}
    >
      <textarea
        ref={area}
        value={text}
        disabled={busy}
        rows={1}
        aria-label={t('transcriptEdit')}
        title={t('transcriptEditHint')}
        onChange={(event) => {
          setText(event.target.value);
          setCursor(event.target.selectionStart);
          setConfirmRemove(false);
        }}
        onSelect={trackCursor}
        onKeyUp={trackCursor}
        onClick={trackCursor}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault();
            onClose();
          } else if (event.key === 'Enter' && !event.shiftKey) {
            event.preventDefault();
            save();
          }
        }}
        className="block w-full resize-none bg-transparent text-sm leading-relaxed text-[var(--af-text)] outline-none"
      />
      <div className="mt-2 flex flex-wrap items-center gap-1.5 text-xs">
        {confirmRemove ? (
          <>
            <span className="mr-auto text-[var(--af-text-2)]">{t('transcriptRemoveConfirm')}</span>
            <button
              type="button"
              disabled={busy}
              onClick={() => void run(onRemove)}
              className="inline-flex items-center gap-1 rounded-md bg-red-500/15 px-2 py-1 font-medium text-red-500 hover:bg-red-500/25 disabled:opacity-50"
            >
              <Trash2 size={13} />
              {t('transcriptRemove')}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => setConfirmRemove(false)}
              className="rounded-md px-2 py-1 text-[var(--af-text-2)] hover:bg-[var(--af-panel-2)]"
            >
              {t('transcriptEditCancel')}
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              disabled={busy}
              onClick={() => setConfirmRemove(true)}
              title={t('transcriptRemove')}
              aria-label={t('transcriptRemove')}
              className="rounded-md p-1 text-[var(--af-text-3)] hover:bg-red-500/15 hover:text-red-500 disabled:opacity-50"
            >
              <Trash2 size={14} />
            </button>
            {onSplit && (
              <button
                type="button"
                disabled={busy || !canSplit}
                onClick={split}
                title={t('transcriptSplitHint')}
                className={quiet}
              >
                <Scissors size={13} />
                {t('transcriptSplit')}
              </button>
            )}
            {onMergeNext && (
              <button
                type="button"
                disabled={busy}
                onClick={mergeNext}
                title={t('transcriptMergeHint')}
                className={quiet}
              >
                <Combine size={13} />
                {t('transcriptMerge')}
              </button>
            )}
            <span className="mr-auto" />
            <button type="button" disabled={busy} onClick={onClose} className={quiet}>
              <X size={13} />
              {t('transcriptEditCancel')}
            </button>
            <button
              type="button"
              disabled={busy || unchanged}
              onClick={save}
              className="inline-flex items-center gap-1 rounded-md bg-[var(--af-accent)] px-2 py-1 font-medium text-[var(--af-accent-contrast)] disabled:opacity-40"
            >
              <Check size={13} />
              {t('transcriptEditSave')}
            </button>
          </>
        )}
      </div>
    </div>
  );
}

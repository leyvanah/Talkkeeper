"use client";

/**
 * Correcting one line of a saved transcript, in place.
 *
 * Enter saves, Escape cancels, Shift+Enter is a new line for the rare line
 * that wants one. Removing asks first: a removed line does not come back, not
 * even when the recording is recognized again — which is the point.
 */

import { useEffect, useRef, useState } from 'react';
import { useTranslations } from 'next-intl';
import { Check, Trash2, X } from 'lucide-react';

export interface LineEditActions {
  /** Save new text for the line. Rejects to keep the editor open. */
  onSave: (text: string) => Promise<void>;
  /** Remove the line. Rejects to keep the editor open. */
  onRemove: () => Promise<void>;
}

export function TranscriptLineEditor({
  initialText,
  onSave,
  onRemove,
  onClose,
}: LineEditActions & {
  initialText: string;
  onClose: () => void;
}) {
  const t = useTranslations('meetingDetails');
  const [text, setText] = useState(initialText);
  const [busy, setBusy] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const area = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const element = area.current;
    if (!element) return;
    element.focus();
    element.setSelectionRange(element.value.length, element.value.length);
  }, []);

  // Grow with the text rather than scroll inside a box.
  useEffect(() => {
    const element = area.current;
    if (!element) return;
    element.style.height = 'auto';
    element.style.height = `${element.scrollHeight}px`;
  }, [text]);

  const trimmed = text.trim();
  const unchanged = trimmed === initialText.trim();

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
          setConfirmRemove(false);
        }}
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
              className="mr-auto rounded-md p-1 text-[var(--af-text-3)] hover:bg-red-500/15 hover:text-red-500 disabled:opacity-50"
            >
              <Trash2 size={14} />
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={onClose}
              className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[var(--af-text-2)] hover:bg-[var(--af-panel-2)]"
            >
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

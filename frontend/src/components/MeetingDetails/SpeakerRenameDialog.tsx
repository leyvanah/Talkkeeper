"use client";

import { useEffect, useState } from 'react';
import { useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Unlink, UserRound } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';
import { useSpeakerWords } from '@/hooks/useSpeakerWords';
import { isOwnerLabel, speakerLabel } from '@/lib/speaker-label';

interface SpeakerRenameDialogProps {
  open: boolean;
  /** The current label being renamed, e.g. "Speaker 2". */
  speaker: string | null;
  meetingId?: string;
  onOpenChange: (open: boolean) => void;
  /** Called after a successful rename/removal so transcript and summary state can refresh. */
  onRenamed?: (rename: { from: string; to: string; count: number; removedName: boolean }) => Promise<void> | void;
}

interface SpeakerRenameResult {
  speaker: string;
  count: number;
  removedName: boolean;
}

function isGeneratedSpeakerLabel(value: string | null): boolean {
  return !!value && /^speaker \d+$/i.test(value.trim());
}

/**
 * Rename one speaker across an entire meeting.
 *
 * Automatic speaker identification is a heuristic — it can mislabel people, and
 * it cannot know who anyone is in meetings recorded before it existed. This lets
 * the user correct it directly, which is both more reliable and more useful than
 * "Speaker 2" ever is.
 *
 * Naming someone "You" marks them as the local user; the transcript then renders
 * that with the display name from Settings. Custom names create/link a durable
 * cross-meeting person. Blank Save and Remove name reverse only this meeting's
 * assignment and ask Rust for a collision-free generated `Speaker N` label.
 */
export function SpeakerRenameDialog({
  open,
  speaker,
  meetingId,
  onOpenChange,
  onRenamed,
}: SpeakerRenameDialogProps) {
  const t = useTranslations('meetingDetails');
  const tc = useTranslations('common');
  const [name, setName] = useState('');
  const [saving, setSaving] = useState(false);
  const [userName, setUserName] = useState('');

  useEffect(() => {
    if (typeof window !== 'undefined') {
      setUserName(localStorage.getItem('meetily_user_name')?.trim() || '');
    }
  }, []);

  const speakerWords = useSpeakerWords();
  // The stored label stays the key; the owner reads it in their own words.
  const shown = (label: string) => speakerLabel(label, userName, speakerWords);

  const canRemoveName = !!speaker && !isGeneratedSpeakerLabel(speaker);

  // Start from an assigned name; a capture label ("You", "Guest", "Speaker N")
  // is a key, not a name, and leaves the field empty.
  const isName = !!speaker && !isGeneratedSpeakerLabel(speaker) && !/^guest$/i.test(speaker.trim()) && !isOwnerLabel(speaker);
  useEffect(() => {
    if (open) setName(isName ? speaker ?? '' : '');
  }, [isName, open, speaker]);

  const submit = async (value: string) => {
    const next = value.trim();
    if (!meetingId || !speaker || (!next && !canRemoveName) || saving) return;

    setSaving(true);
    try {
      const result = await invoke<SpeakerRenameResult>('rename_meeting_speaker', {
        meetingId,
        from: speaker,
        to: next,
      });
      if (result.removedName) {
        toast.success(t('speakerNameRemovedTitle'), {
          description: t('speakerNameRemovedDescription', { speaker: shown(result.speaker), count: result.count }),
        });
      } else {
        toast.success(t('speakerRenamedTitle', { name: shown(result.speaker) }), {
          description: t('speakerRenamedDescription', { count: result.count }),
        });
      }
      onOpenChange(false);
      await onRenamed?.({
        from: speaker,
        to: result.speaker,
        count: result.count,
        removedName: result.removedName,
      });
    } catch (e) {
      toast.error(t('speakerRenameFailed'), {
        description: e instanceof Error ? e.message : String(e),
      });
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent aria-describedby={undefined} className="sm:max-w-md">
        <DialogTitle className="flex items-center gap-2 text-base">
          <UserRound size={18} className="text-blue-500" />
          {t('speakerWhoIs', { speaker: speaker ? shown(speaker) : '' })}
        </DialogTitle>

        <div className="mt-2 space-y-3">
          <p className="text-sm text-gray-500">
            {t.rich('speakerRenameHint', {
              name: () => <strong>{speaker ? shown(speaker) : ''}</strong>,
            })}
          </p>

          <input
            type="text"
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                submit(name);
              }
            }}
            placeholder={t('speakerNamePlaceholder')}
            className="w-full rounded-md border border-[var(--af-border,#d1d5db)] bg-[var(--af-panel-2,#fff)] px-3 py-2 text-sm text-[var(--af-text,#111827)] outline-none focus:ring-2 focus:ring-blue-500"
          />

          {/* One-click "this is me" — the common case, and it also teaches the
              offline diarization pass which speaker is the user. */}
          <button
            type="button"
            onClick={() => submit('You')}
            disabled={saving}
            className="flex w-full items-center gap-2 rounded-md border border-[var(--af-border,#e5e7eb)] px-3 py-2 text-left text-sm text-gray-600 transition-colors hover:border-blue-400 hover:text-blue-500"
          >
            <UserRound size={15} />
            {userName ? t('speakerThisIsMeNamed', { name: userName }) : t('speakerThisIsMe')}
          </button>

          {canRemoveName && (
            <button
              type="button"
              onClick={() => submit('')}
              disabled={saving}
              className="flex w-full items-center gap-2 rounded-md border border-red-500/30 px-3 py-2 text-left text-sm text-red-500 transition-colors hover:border-red-500/60 hover:bg-red-500/10"
            >
              <Unlink size={15} />
              {t('speakerRemoveName')}
            </button>
          )}
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <Button variant="outline" size="sm" onClick={() => onOpenChange(false)}>
            {tc('cancel')}
          </Button>
          <Button
            size="sm"
            className="bg-blue-600 text-white hover:bg-blue-700"
            disabled={(!name.trim() && !canRemoveName) || saving}
            onClick={() => submit(name)}
          >
            {saving ? t('speakerSaving') : name.trim() ? t('speakerRename') : t('speakerRemoveName')}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

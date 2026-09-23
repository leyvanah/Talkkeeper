"use client";

/**
 * The speaker's name over a line, and what can be done from it.
 *
 * With corrections allowed, the name opens a menu: give this line to another
 * speaker (one the meeting has, or a new one), or rename the speaker across
 * the whole meeting. Without them it is the rename button it always was.
 */

import { useTranslations } from 'next-intl';
import { Pencil } from 'lucide-react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';

export interface SpeakerChoice {
  /** Speakers the meeting has (raw labels). */
  existing: string[];
  /** A label no line has yet, offered as a new speaker. */
  fresh: string;
  /** Give the line to this speaker. */
  choose: (speaker: string) => Promise<void>;
}

export function SpeakerLabelMenu({
  speaker,
  label,
  displayOf,
  className,
  onRename,
  choice,
}: {
  /** The line's speaker (raw label). */
  speaker: string;
  /** How it reads on screen. */
  label: string;
  /** How any raw label reads on screen. */
  displayOf: (speaker: string) => string;
  className: string;
  onRename?: (speaker: string) => void;
  choice?: SpeakerChoice;
}) {
  const t = useTranslations('recording');
  const tm = useTranslations('meetingDetails');

  if (!choice) {
    return onRename ? (
      <button
        type="button"
        onClick={() => onRename(speaker)}
        title={t('renameSpeakerTitle', { speaker })}
        className={`${className} rounded hover:underline`}
      >
        {label}
      </button>
    ) : (
      <span className={className}>{label}</span>
    );
  }

  const options = choice.existing.includes(choice.fresh)
    ? choice.existing
    : [...choice.existing, choice.fresh];

  return (
    // Not modal: the rename item opens a dialog, and a modal menu closing
    // underneath it would leave the page unclickable.
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          title={tm('speakerMenuHint')}
          className={`${className} rounded hover:underline`}
        >
          {label}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-[14rem]">
        <DropdownMenuLabel className="text-xs font-normal text-[var(--af-text-3)]">
          {tm('speakerMenuWhoSays')}
        </DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={speaker}
          onValueChange={(next) => {
            if (next !== speaker) void choice.choose(next).catch(() => undefined);
          }}
        >
          {options.map((option) => (
            <DropdownMenuRadioItem key={option} value={option}>
              {option === choice.fresh && !choice.existing.includes(option)
                ? tm('transcriptNewSpeaker', { speaker: option })
                : displayOf(option)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
        {onRename && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => onRename(speaker)}>
              <Pencil className="mr-2 h-3.5 w-3.5" />
              {tm('speakerMenuRename', { speaker: label })}
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

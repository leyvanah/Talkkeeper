/**
 * What a speaker label reads as, on screen and in text that leaves the app.
 *
 * The database keeps the capture labels as keys — "You" for the local
 * microphone, "Guest" for a single voice on the other end, "Speaker N" for one
 * of several — because diarization and the speaker roles are built on them.
 * They are never shown as they are: this turns them into the interface's own
 * words. A name someone gave a speaker is shown as given.
 *
 * The owner has one form on screen, "<name> (You)" in the interface language,
 * and just their name in an export or a copy: "(You)" means nothing to whoever
 * reads the text later.
 */

export interface SpeakerWords {
  /** The owner when no name is set: "Вы" / "You". */
  you: string
  /** The owner with a name, on screen: "<name> (Вы)". */
  youWithName: (name: string) => string
  /** A single voice on the other end: "Гость" / "Guest". */
  guest: string
  /** One of several voices: "Спикер N" / "Speaker N". */
  numbered: (number: number) => string
}

/** Where the label goes: the screen, or text that leaves the app. */
export type SpeakerLabelUse = 'screen' | 'export'

const NUMBERED = /^speaker\s+(\d+)$/i

export function isOwnerLabel(label: string): boolean {
  const trimmed = label.trim()
  return /^you\b/i.test(trimmed) || /\(\s*you\s*\)$/i.test(trimmed)
}

export function speakerLabel(
  raw: string,
  userName: string,
  words: SpeakerWords,
  use: SpeakerLabelUse = 'screen',
): string {
  // Both at once: name each voice, rather than letting the first stand for
  // the whole line.
  if (raw.includes(' + ')) {
    return raw
      .split(' + ')
      .map((part) => speakerLabel(part.trim(), userName, words, use))
      .join(' + ')
  }
  const label = raw.trim()
  const name = userName.trim()
  if (isOwnerLabel(label)) {
    if (!name) return words.you
    return use === 'export' ? name : words.youWithName(name)
  }
  if (/^guest$/i.test(label)) return words.guest
  const numbered = NUMBERED.exec(label)
  if (numbered) return words.numbered(Number(numbered[1]))
  return raw
}

/**
 * The stored key a search term stands for, when the term is how the interface
 * shows one: "вы" finds the owner's lines, "гость" the guest's. `null` when the
 * term is not a shown label.
 */
export function speakerKeyForTerm(term: string, words: SpeakerWords): string | null {
  const query = term.trim().toLowerCase()
  if (!query) return null
  if (query === words.you.toLowerCase()) return 'You'
  if (query === words.guest.toLowerCase()) return 'Guest'
  const numberedPrefix = words.numbered(0).replace(/0\s*$/, '').trim().toLowerCase()
  if (numberedPrefix && query.startsWith(numberedPrefix)) {
    const number = query.slice(numberedPrefix.length).trim()
    if (/^\d+$/.test(number)) return `Speaker ${Number(number)}`
  }
  return null
}

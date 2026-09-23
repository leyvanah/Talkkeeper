/**
 * Who a line of a transcript can be given to.
 *
 * Every speaker the meeting already has — a line said "together" counts for
 * each of its voices, not as a speaker of its own — plus the local user, who
 * is always there, and one new speaker for a voice the recognizer never told
 * apart.
 */

const NUMBERED = /^speaker\s+(\d+)$/i

function isLocal(label: string): boolean {
  return /^you\b/i.test(label) || /\(\s*you\s*\)$/i.test(label)
}

export interface SpeakerChoices {
  /** Existing speakers, the local user first, the rest in reading order. */
  existing: string[]
  /** A label no line has yet: the next "Speaker N". */
  fresh: string
}

export function speakerChoices(labels: Array<string | undefined>): SpeakerChoices {
  const seen = new Map<string, string>()
  for (const label of labels) {
    for (const part of (label ?? '').split(' + ')) {
      const trimmed = part.trim()
      if (trimmed && !seen.has(trimmed.toLowerCase())) seen.set(trimmed.toLowerCase(), trimmed)
    }
  }
  const all = Array.from(seen.values())
  const local = all.filter(isLocal)
  const others = all.filter((label) => !isLocal(label))
  const existing = [...(local.length ? local : ['You']), ...others]

  const highest = all.reduce((max, label) => {
    const match = NUMBERED.exec(label)
    return match ? Math.max(max, Number(match[1])) : max
  }, 0)
  return { existing, fresh: `Speaker ${highest + 1}` }
}

/**
 * Recognises the line a summary model writes into a section it has nothing
 * for, so the meeting page does not show it as an action item or a topic.
 *
 * The final-report prompt asks for a single em dash in an empty section
 * (`EMPTY_SECTION_MARKER`, mirrored in `src-tauri/src/summary/processor.rs`):
 * the report is written in English and then translated, and a dash survives
 * translation where a sentence comes back reworded in every language.
 * Summaries made before that asked for "None noted in this section.", and
 * small models still answer with a bare "None" or "N/A" now and then; those
 * are recognised too.
 */

/** What the prompt asks for in an empty section. Keep in step with the Rust prompt. */
export const EMPTY_SECTION_MARKER = '—'

/** Whole-line answers that mean "nothing here", compared without case or punctuation. */
const NOTHING_HERE = new Set([
  'none noted in this section',
  'none noted',
  'none',
  'n/a',
  'na',
  'нет',
  'н/а',
])

export function isEmptySectionPlaceholder(text: string): boolean {
  const bare = text
    .replace(/[*_`"'“”«»()[\]]/g, '')
    .replace(/^\s*(?:[-+•]\s+|\d+[.)]\s+)/, '')
    .trim()
  // Only dashes (the marker, or a model's take on it) and nothing else.
  if (/^[-–—\s]*$/.test(bare)) return true
  const words = bare.replace(/[.!]+$/, '').trim().toLowerCase()
  return NOTHING_HERE.has(words)
}

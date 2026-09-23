/**
 * Where a line is cut when the cursor stands at `cursor`.
 *
 * A cut never falls inside a word: a cursor in the middle of one moves back
 * to where the word starts, so the whole word goes with the second half —
 * the half whose speaker is about to begin it. Only a word that starts the
 * text moves the cut forward to where it ends.
 */
export function splitPoint(text: string, cursor: number): number {
  const isSpace = (character: string) => /\s/.test(character)
  if (cursor <= 0 || cursor >= text.length) return cursor
  if (isSpace(text[cursor - 1]) || isSpace(text[cursor])) return cursor
  let back = cursor
  while (back > 0 && !isSpace(text[back - 1])) back -= 1
  if (back > 0) return back
  let forward = cursor
  while (forward < text.length && !isSpace(text[forward])) forward += 1
  return forward
}

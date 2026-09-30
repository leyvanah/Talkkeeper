/**
 * Shape and motion of the compact recording bar docked to the top of a screen.
 *
 * Docked, the bar is a tab hanging from the screen edge: its bottom corners
 * are rounded as usual, its top corners curve outwards into the edge, the way
 * a drop clings to a surface. Away from the cursor it slides up behind the
 * edge, leaving a sliver, and fades; as the cursor comes near it slides back.
 */

/** How far (px) from the bar the cursor starts drawing it out. */
export const REVEAL_RANGE = 140;

/**
 * How much of the bar is out, from 0 (tucked away) to 1 (all of it), for a
 * cursor `distance` px from the window. `held` keeps it out regardless: the
 * cursor has only just left, or the owner is typing a note.
 */
export function revealFor(distance: number, held: boolean): number {
  if (held || distance <= 0) return 1;
  const near = Math.min(1, Math.max(0, 1 - distance / REVEAL_RANGE));
  // Gentle at first, quick once the cursor is really coming for it.
  return near * near;
}

export interface TabPaths {
  /** The filled tab, its top edge along the screen edge. */
  fill: string;
  /** Its outline, without the top edge: the screen edge draws that one. */
  outline: string;
}

/**
 * The docked tab in a `width` × `height` box: the body stands `ear` px in from
 * each side, where the top corners curve outwards with radius `ear`; the
 * bottom corners are rounded with `radius`. Lines sit on half pixels so a 1px
 * outline stays crisp.
 */
export function dockedTabPaths(width: number, height: number, ear: number, radius: number): TabPaths {
  const e = ear + 0.5;
  const b = height - 0.5;
  const r = Math.min(radius, b - e, (width - 2 * e) / 2);
  const left = e;
  const right = width - e;
  const fill = [
    `M0 0 H${width}`,
    `A${e} ${e} 0 0 0 ${right} ${e}`,
    `V${b - r}`,
    `A${r} ${r} 0 0 1 ${right - r} ${b}`,
    `H${left + r}`,
    `A${r} ${r} 0 0 1 ${left} ${b - r}`,
    `V${e}`,
    `A${e} ${e} 0 0 0 0 0 Z`,
  ].join(' ');
  const outline = [
    `M0 0`,
    `A${e} ${e} 0 0 1 ${left} ${e}`,
    `V${b - r}`,
    `A${r} ${r} 0 0 0 ${left + r} ${b}`,
    `H${right - r}`,
    `A${r} ${r} 0 0 0 ${right} ${b - r}`,
    `V${e}`,
    `A${e} ${e} 0 0 1 ${width} 0`,
  ].join(' ');
  return { fill, outline };
}

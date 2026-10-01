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

/** How far (px) the bar has to come away from the edge to be all pill again. */
export const PEEL_DISTANCE = 6;

/**
 * How far the bar has come away from the screen edge, from 0 (against it, a
 * tab) to 1 (free, a pill). A bar pulled down gives a little before it tears
 * off; the moment there is daylight between it and the edge it must stop
 * looking attached.
 */
export function peelFor(docked: boolean, edgeGap: number): number {
  if (!docked) return 1;
  return Math.min(1, Math.max(0, edgeGap / PEEL_DISTANCE));
}

export interface BarPaths {
  /** The filled shape. */
  fill: string;
  /** Its outline, all but the top edge. */
  outline: string;
  /** The top edge: drawn by the screen edge while docked, by the bar when free. */
  top: string;
}

/** A quarter circle as a cubic Bézier: control points this far along the tangents. */
const KAPPA = 0.5523;

const lerp = (from: number, to: number, t: number) => from + (to - from) * t;
const n = (value: number) => String(Math.round(value * 100) / 100);
const pt = (x: number, y: number) => `${n(x)} ${n(y)}`;

/**
 * The bar in a `width` × `height` box, `peel` of the way from a tab hanging
 * off the screen edge (0) to a free pill (1).
 *
 * The tab's top corners curve outwards into the edge, `ear` px each side; its
 * bottom corners are rounded with `radius`. The pill stands `ear` px in from
 * each side and is rounded all round. Both are drawn with the same sequence of
 * lines and cubic curves, so every point can be eased from one to the other
 * and the corners peel off the edge rather than switching shape. Lines sit on
 * half pixels so a 1px outline stays crisp.
 */
export function barPaths(width: number, height: number, ear: number, radius: number, peel: number): BarPaths {
  const p = Math.min(1, Math.max(0, peel));
  const left = ear + 0.5;
  const right = width - ear - 0.5;
  const bottom = height - 0.5;
  const pill = (bottom - 0.5) / 2;
  const topY = lerp(0, 0.5, p);
  // Bottom corners grow from the tab's radius to the pill's half-height.
  const rb = lerp(Math.min(radius, pill), pill, p);

  // Top-left corner, from the side (s) up to the top edge (t). Docked it is a
  // concave quarter circle of radius `left` out to the screen's corner; free,
  // a convex one of the pill's radius.
  const s = { x: left, y: lerp(left, topY + pill, p) };
  const t = { x: lerp(0, left + pill, p), y: topY };
  const c1 = { x: left, y: lerp(left - KAPPA * left, topY + pill - KAPPA * pill, p) };
  const c2 = { x: lerp(KAPPA * left, left + pill - KAPPA * pill, p), y: topY };
  const mirror = (point: { x: number; y: number }) => ({ x: width - point.x, y: point.y });
  const [sr, tr, c1r, c2r] = [s, t, c1, c2].map(mirror);
  const k = KAPPA * rb;

  const fill = [
    `M${pt(t.x, t.y)}`,
    `L${pt(tr.x, tr.y)}`,
    `C${pt(c2r.x, c2r.y)} ${pt(c1r.x, c1r.y)} ${pt(sr.x, sr.y)}`,
    `L${pt(right, bottom - rb)}`,
    `C${pt(right, bottom - rb + k)} ${pt(right - rb + k, bottom)} ${pt(right - rb, bottom)}`,
    `L${pt(left + rb, bottom)}`,
    `C${pt(left + rb - k, bottom)} ${pt(left, bottom - rb + k)} ${pt(left, bottom - rb)}`,
    `L${pt(s.x, s.y)}`,
    `C${pt(c1.x, c1.y)} ${pt(c2.x, c2.y)} ${pt(t.x, t.y)}`,
    'Z',
  ].join(' ');
  const outline = [
    `M${pt(t.x, t.y)}`,
    `C${pt(c2.x, c2.y)} ${pt(c1.x, c1.y)} ${pt(s.x, s.y)}`,
    `L${pt(left, bottom - rb)}`,
    `C${pt(left, bottom - rb + k)} ${pt(left + rb - k, bottom)} ${pt(left + rb, bottom)}`,
    `L${pt(right - rb, bottom)}`,
    `C${pt(right - rb + k, bottom)} ${pt(right, bottom - rb + k)} ${pt(right, bottom - rb)}`,
    `L${pt(sr.x, sr.y)}`,
    `C${pt(c1r.x, c1r.y)} ${pt(c2r.x, c2r.y)} ${pt(tr.x, tr.y)}`,
  ].join(' ');
  const top = `M${pt(t.x, t.y)} L${pt(tr.x, tr.y)}`;
  return { fill, outline, top };
}

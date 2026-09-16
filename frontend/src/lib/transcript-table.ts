/**
 * Laying a transcript out along a time axis: one column per side, every line
 * placed at the moment it starts.
 *
 * The two sides are recorded on separate tracks with one clock, so an
 * interruption is two lines whose times overlap. Here each line sits exactly at
 * its start on the ruler, so an interruption appears beside the part of the
 * other person's line it cut into, not merely in the same row.
 *
 * ## An elastic ruler
 *
 * A strictly proportional axis cannot hold text: a sentence can take more room
 * on screen than its seconds are given. So the axis is piecewise linear. Every
 * line start is an anchor; between two anchors time runs at `pxPerSecond`
 * unless the text above needs more room, in which case that stretch of the
 * ruler is lengthened. The ruler is drawn with the same mapping, so a line is
 * always on the tick of its own second — the ticks are simply further apart
 * where there is a lot to read.
 *
 * Pure on purpose: no React, no Tauri, so it can be tested with plain node.
 */

export type TimelineColumn = 'host' | 'client' | 'both';

export interface TimelineLine {
  id: string;
  /** Seconds from the start of the recording. */
  start: number;
  /** Seconds; a line with no known end is treated as a point in time. */
  end?: number;
  text: string;
  speaker?: string;
  confidence?: number;
}

export interface PlacedLine {
  line: TimelineLine;
  column: TimelineColumn;
  /** Pixels from the top of the timeline. */
  top: number;
  height: number;
}

/** A point where time and position are pinned to each other. */
export interface Anchor {
  t: number;
  y: number;
}

export interface TimelineLayout {
  placed: PlacedLine[];
  /** Ascending in both `t` and `y`. */
  anchors: Anchor[];
  height: number;
  pxPerSecond: number;
}

export interface LayoutOptions {
  pxPerSecond: number;
  /** Space kept between two lines in the same column. */
  gap: number;
  /** Space above time zero and below the last line. */
  padding: number;
}

/**
 * Starts closer than this are one instant. Recorded times are no finer than a
 * millisecond, and without it two sums that should be equal (0.7 * 9 and
 * 0.9 * 7) would be two anchors a rounding error apart.
 */
const SAME_INSTANT = 0.0005;

function endOf(line: TimelineLine): number {
  return Math.max(line.start, line.end ?? line.start);
}

function columnsOf(column: TimelineColumn): Array<'host' | 'client'> {
  return column === 'both' ? ['host', 'client'] : [column];
}

export function layoutTimeline(
  lines: TimelineLine[],
  sideOf: (speaker?: string) => TimelineColumn,
  heightOf: (line: TimelineLine) => number,
  { pxPerSecond, gap, padding }: LayoutOptions,
): TimelineLayout {
  const sorted = [...lines].sort((a, b) => a.start - b.start || endOf(a) - endOf(b));
  const anchors: Anchor[] = [{ t: 0, y: padding }];
  const placed: PlacedLine[] = [];
  // Where each column is free from, in pixels.
  const free = { host: padding, client: padding };
  // Lines pinned to the newest anchor, which may still have to move down.
  let atAnchor: PlacedLine[] = [];

  for (const line of sorted) {
    const column = sideOf(line.speaker);
    const height = heightOf(line);
    const last = anchors[anchors.length - 1];
    const start = line.start - last.t < SAME_INSTANT ? last.t : line.start;
    const cols = columnsOf(column);
    // Below whatever is already in the column, with a gap once there is
    // something to keep a gap from.
    const needed = Math.max(...cols.map((c) => free[c] + (free[c] > padding ? gap : 0)));

    let top: number;
    if (
      start === last.t &&
      atAnchor.some((entry) => columnsOf(entry.column).some((c) => cols.includes(c)))
    ) {
      // Two lines of one column starting at the same instant cannot both sit
      // on it; the later one goes directly underneath.
      top = needed;
    } else if (start === last.t) {
      // Starts together with the lines already on this anchor, in another
      // column: they share one position, so if this one needs more room they
      // all move down.
      if (needed > last.y) {
        const shift = needed - last.y;
        last.y = needed;
        for (const entry of atAnchor) {
          entry.top += shift;
          for (const c of columnsOf(entry.column)) {
            free[c] = Math.max(free[c], entry.top + entry.height);
          }
        }
      }
      top = last.y;
    } else {
      const natural = last.y + (start - last.t) * pxPerSecond;
      anchors.push({ t: start, y: Math.max(natural, needed) });
      atAnchor = [];
      top = anchors[anchors.length - 1].y;
    }

    const entry: PlacedLine = { line, column, top, height };
    placed.push(entry);
    atAnchor.push(entry);
    for (const c of columnsOf(column)) {
      free[c] = Math.max(free[c], top + height);
    }
  }

  // Close the axis at the last moment anyone was speaking, and below the last
  // line on screen, whichever is further.
  const last = anchors[anchors.length - 1];
  const spokenUntil = sorted.reduce((latest, line) => Math.max(latest, endOf(line)), last.t);
  const finalY = Math.max(last.y + (spokenUntil - last.t) * pxPerSecond, free.host, free.client);
  if (finalY > last.y) {
    // The anchor the last lines sit on must not move; the extra room belongs
    // to the time after it.
    const finalT = Math.max(spokenUntil, last.t + (finalY - last.y) / pxPerSecond);
    anchors.push({ t: finalT, y: finalY });
  }

  return {
    placed,
    anchors,
    height: anchors[anchors.length - 1].y + padding,
    pxPerSecond,
  };
}

/** Index of the last anchor at or before `value` on the given axis. */
function anchorBefore(anchors: Anchor[], value: number, axis: 't' | 'y'): number {
  let low = 0;
  let high = anchors.length - 1;
  while (low < high) {
    const mid = (low + high + 1) >> 1;
    if (anchors[mid][axis] <= value) low = mid;
    else high = mid - 1;
  }
  return low;
}

/** Where a moment of the recording is on screen. */
export function yAt(layout: TimelineLayout, t: number): number {
  const { anchors, pxPerSecond } = layout;
  const i = anchorBefore(anchors, t, 't');
  const a = anchors[i];
  const b = anchors[i + 1];
  if (!b || t <= a.t) return a.y + Math.max(0, t - a.t) * pxPerSecond;
  return a.y + ((t - a.t) / (b.t - a.t)) * (b.y - a.y);
}

/** Which moment of the recording is at a position on screen. */
export function timeAt(layout: TimelineLayout, y: number): number {
  const { anchors, pxPerSecond } = layout;
  const i = anchorBefore(anchors, y, 'y');
  const a = anchors[i];
  const b = anchors[i + 1];
  if (y <= a.y) {
    // Above the first anchor, or on a stretch where several anchors share a
    // position: the earliest time there.
    return Math.max(0, a.t - (a.y - y) / pxPerSecond);
  }
  if (!b) return a.t + (y - a.y) / pxPerSecond;
  if (b.y === a.y) return a.t;
  return a.t + ((y - a.y) / (b.y - a.y)) * (b.t - a.t);
}

export interface Tick {
  t: number;
  y: number;
  /** `label` ticks carry a time, `major` ones a long mark, the rest a short one. */
  kind: 'label' | 'major' | 'minor';
}

/**
 * Ruler marks between two positions on screen.
 *
 * Every second gets a mark. Labels are spaced so they never crowd: every five
 * seconds at the default scale, every second when zoomed in, and tenths of a
 * second get their own marks once there is room for them.
 */
export function ticksBetween(layout: TimelineLayout, fromY: number, toY: number): Tick[] {
  const pps = layout.pxPerSecond;
  const step = pps >= 64 ? 0.1 : 1;
  const labelEvery = pps >= 64 ? 1 : pps >= 32 ? 5 : 10;
  const from = Math.max(0, Math.floor(timeAt(layout, fromY) / step) * step);
  const to = timeAt(layout, toY);
  const ticks: Tick[] = [];
  // Counting in steps rather than adding floats keeps 0.1 from drifting.
  const firstIndex = Math.round(from / step);
  const lastIndex = Math.ceil(to / step);
  for (let index = firstIndex; index <= lastIndex; index++) {
    const t = Math.round(index * step * 10) / 10;
    const whole = Number.isInteger(t);
    const kind: Tick['kind'] =
      whole && t % labelEvery === 0 ? 'label' : whole ? 'major' : 'minor';
    ticks.push({ t, y: yAt(layout, t), kind });
  }
  return ticks;
}

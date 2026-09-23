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
  /** When each word was said, where the recognizer reported it. */
  words?: TimedWord[];
  /** The stored lines this one shows, when several were merged. */
  ids?: string[];
  /** A person corrected it. */
  edited?: boolean;
  /**
   * The text as stored, when `text` shows something else (the timed words).
   * A correction starts from this, so nothing the timings left out is lost.
   */
  stored?: string;
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
  /**
   * Ascending in both `t` and `y`. Their `t` is shown time — the recording's
   * time with its long pauses shortened (see `QuietOptions`).
   */
  anchors: Anchor[];
  height: number;
  pxPerSecond: number;
  /** Recording time to shown time; the identity when no pause is shortened. */
  warp: Warp;
  /** The pauses that were shortened, where they are on screen. */
  quiet: QuietStretch[];
}

/**
 * Shortening the stretches where nobody says anything.
 *
 * A proportional axis gives a silence as much room as speech, and a meeting
 * has plenty of it: the minute before anyone speaks, someone thinking. A
 * pause no longer than `keep` seconds stays as it is — it is part of how a
 * conversation reads. A longer one is shown as `keep` plus `rate` of the rest,
 * and never as more than `most` seconds. Both columns share the one axis, so an
 * interruption still sits beside the words it cut into.
 */
export interface QuietOptions {
  keep: number;
  rate: number;
  most: number;
}

/** A shortened pause: its real times and its place on screen. */
export interface QuietStretch {
  from: number;
  to: number;
  top: number;
  bottom: number;
}

/** Matching points of recording time and shown time, ascending in both. */
export interface Warp {
  real: number[];
  shown: number[];
}

export interface LayoutOptions {
  pxPerSecond: number;
  /** Space kept between two lines in the same column. */
  gap: number;
  /** Space above time zero and below the last line. */
  padding: number;
  /** Shorten long pauses; without it, time is shown as recorded. */
  quiet?: QuietOptions;
}

/** When someone is speaking: the words where they have times, else the line. */
function speechOf(lines: TimelineLine[]): Array<[number, number]> {
  const spans: Array<[number, number]> = [];
  for (const line of lines) {
    if (line.words?.length) {
      for (const word of line.words) spans.push([word.s, Math.max(word.s, word.e)]);
    } else {
      spans.push([line.start, endOf(line)]);
    }
  }
  return spans.sort((a, b) => a[0] - b[0]);
}

/** The pauses of `quiet` and how long each is shown for. */
export function buildWarp(lines: TimelineLine[], quiet?: QuietOptions): Warp {
  const warp: Warp = { real: [0], shown: [0] };
  if (!quiet) return warp;
  let spokenUntil = 0;
  for (const [start, end] of speechOf(lines)) {
    const pause = start - spokenUntil;
    if (pause > quiet.keep) {
      const shown = Math.min(pause, quiet.most, quiet.keep + (pause - quiet.keep) * quiet.rate);
      const at = toShown(warp, spokenUntil);
      warp.real.push(spokenUntil, start);
      warp.shown.push(at, at + shown);
    }
    spokenUntil = Math.max(spokenUntil, end);
  }
  return warp;
}

/** Index of the last point at or before `value` in an ascending list. */
function pointBefore(points: number[], value: number): number {
  let low = 0;
  let high = points.length - 1;
  while (low < high) {
    const mid = (low + high + 1) >> 1;
    if (points[mid] <= value) low = mid;
    else high = mid - 1;
  }
  return low;
}

/** Map through piecewise-linear points; past the last one, a second is a second. */
function through(from: number[], to: number[], value: number): number {
  const i = pointBefore(from, value);
  const next = i + 1;
  if (next >= from.length || value <= from[i]) return to[i] + (value - from[i]);
  if (from[next] === from[i]) return to[i];
  return to[i] + ((value - from[i]) / (from[next] - from[i])) * (to[next] - to[i]);
}

export function toShown(warp: Warp, t: number): number {
  return through(warp.real, warp.shown, t);
}

export function toReal(warp: Warp, t: number): number {
  return through(warp.shown, warp.real, t);
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
  { pxPerSecond, gap, padding, quiet }: LayoutOptions,
): TimelineLayout {
  const sorted = [...lines].sort((a, b) => a.start - b.start || endOf(a) - endOf(b));
  // Everything below is placed in shown time.
  const warp = buildWarp(sorted, quiet);
  const shown = (t: number) => toShown(warp, t);
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
    const lineStart = shown(line.start);
    const start = lineStart - last.t < SAME_INSTANT ? last.t : lineStart;
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
  const spokenUntil = sorted.reduce((latest, line) => Math.max(latest, shown(endOf(line))), last.t);
  const finalY = Math.max(last.y + (spokenUntil - last.t) * pxPerSecond, free.host, free.client);
  if (finalY > last.y) {
    // The anchor the last lines sit on must not move; the extra room belongs
    // to the time after it.
    const finalT = Math.max(spokenUntil, last.t + (finalY - last.y) / pxPerSecond);
    anchors.push({ t: finalT, y: finalY });
  }

  const layout: TimelineLayout = {
    placed,
    anchors,
    height: anchors[anchors.length - 1].y + padding,
    pxPerSecond,
    warp,
    quiet: [],
  };
  // The warp's points after the first come in pairs, one pair per pause.
  for (let i = 1; i + 1 < warp.real.length; i += 2) {
    const from = warp.real[i];
    const to = warp.real[i + 1];
    layout.quiet.push({ from, to, top: yAt(layout, from), bottom: yAt(layout, to) });
  }
  return layout;
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
export function yAt(layout: TimelineLayout, seconds: number): number {
  const { anchors, pxPerSecond } = layout;
  const t = toShown(layout.warp, seconds);
  const i = anchorBefore(anchors, t, 't');
  const a = anchors[i];
  const b = anchors[i + 1];
  if (!b || t <= a.t) return a.y + Math.max(0, t - a.t) * pxPerSecond;
  return a.y + ((t - a.t) / (b.t - a.t)) * (b.y - a.y);
}

/** Which moment of the recording is at a position on screen. */
export function timeAt(layout: TimelineLayout, y: number): number {
  return Math.max(0, toReal(layout.warp, shownTimeAt(layout, y)));
}

function shownTimeAt(layout: TimelineLayout, y: number): number {
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

/** Closest two ruler marks may be, and two labelled ones, in pixels. */
const MIN_TICK_GAP = 3;
const MIN_LABEL_GAP = 24;

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
 *
 * A shortened pause gets no marks inside it — its seconds are squeezed too
 * tight to tell apart; the pause is drawn as one stretch instead. Near its
 * edges a mark too close to the one before is left out, and a label too close
 * to the one before loses its text.
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
  let lastY = -Infinity;
  let lastLabelY = -Infinity;
  let pause = 0;
  for (let index = firstIndex; index <= lastIndex; index++) {
    const t = Math.round(index * step * 10) / 10;
    while (pause < layout.quiet.length && layout.quiet[pause].to <= t) pause++;
    const inside = layout.quiet[pause];
    if (inside && inside.from < t) {
      // Skip to the end of the pause; the loop's own step lands on it.
      index = Math.max(index, Math.ceil(inside.to / step) - 1);
      continue;
    }
    const y = yAt(layout, t);
    if (y - lastY < MIN_TICK_GAP) continue;
    const whole = Number.isInteger(t);
    let kind: Tick['kind'] =
      whole && t % labelEvery === 0 ? 'label' : whole ? 'major' : 'minor';
    if (kind === 'label' && y - lastLabelY < MIN_LABEL_GAP) kind = 'major';
    if (kind === 'label') lastLabelY = y;
    lastY = y;
    ticks.push({ t, y, kind });
  }
  return ticks;
}

/**
 * A line's text cut into pieces that can each sit near the moment they were
 * said.
 *
 * Without word timings the best estimate is that speech runs at an even pace,
 * so a piece is placed by how much of the text comes before it. Pieces are
 * sentences, and long sentences are cut further at a comma or a space, so a
 * minute-long monologue is spread over its minute rather than bunched at its
 * start.
 */
export function splitForTimeline(text: string, maxChars = 90): string[] {
  const sentences = text
    .replace(/\s+/g, ' ')
    .trim()
    .split(/(?<=[.!?…])\s+/)
    .filter(Boolean);
  const pieces: string[] = [];
  for (const sentence of sentences) {
    let rest = sentence;
    while (rest.length > maxChars) {
      const window = rest.slice(0, maxChars + 1);
      const comma = window.lastIndexOf(', ');
      const space = window.lastIndexOf(' ');
      // A comma in the second half of the window reads best; otherwise the
      // last space; a single unbroken word stays whole.
      const cut = comma > maxChars / 2 ? comma + 1 : space > 0 ? space : -1;
      if (cut <= 0) break;
      pieces.push(rest.slice(0, cut).trim());
      rest = rest.slice(cut).trim();
    }
    if (rest) pieces.push(rest);
  }
  return pieces.length ? pieces : [text];
}

/**
 * Where each piece goes inside its line, in pixels from where the text starts.
 *
 * `offsetAt(fraction)` says how far down the line's own span a fraction of its
 * duration is — the elastic ruler, not a straight proportion. A piece goes at
 * the position of the fraction of text before it, or directly under the piece
 * above if that is further down.
 *
 * With `bottom`, nothing may end below it: pieces that would are moved up —
 * the last ones first — just far enough for everything to fit.
 */
export function placePieces(
  pieces: string[],
  heights: number[],
  offsetAt: (fraction: number) => number,
  gap = 0,
  bottom = Infinity,
): number[] {
  const total = pieces.reduce((sum, piece) => sum + piece.length, 0) || 1;
  const tops: number[] = [];
  let before = 0;
  let free = 0;
  pieces.forEach((piece, index) => {
    const top = Math.max(offsetAt(before / total), free);
    tops.push(top);
    free = top + (heights[index] ?? 0) + gap;
    before += piece.length;
  });
  let limit = bottom;
  for (let index = tops.length - 1; index >= 0; index--) {
    tops[index] = Math.max(0, Math.min(tops[index], limit - (heights[index] ?? 0)));
    limit = tops[index] - gap;
  }
  return tops;
}

/** A word with the moment it was said, as the recognizer reported it. */
export interface TimedWord {
  w: string;
  s: number;
  e: number;
}

/** A piece of a line whose words have times. */
export interface TimedPiece {
  text: string;
  /** Seconds of the recording: when the piece's first word began. */
  start: number;
  words: TimedWord[];
}

/**
 * Cut timed words into pieces by the same rule as `splitForTimeline` — at the
 * end of a sentence, and before a piece grows past `maxChars` — so a timed line
 * reads the same as an untimed one, only placed by the clock.
 */
export function piecesFromWords(words: TimedWord[], maxChars = 90): TimedPiece[] {
  const pieces: TimedPiece[] = [];
  let current: TimedWord[] = [];
  let length = 0;
  const flush = () => {
    if (!current.length) return;
    pieces.push({ text: current.map((word) => word.w).join(' '), start: current[0].s, words: current });
    current = [];
    length = 0;
  };
  for (const word of words) {
    if (current.length && length + 1 + word.w.length > maxChars) flush();
    current.push(word);
    length += (length ? 1 : 0) + word.w.length;
    if (/[.!?…]$/.test(word.w)) flush();
  }
  flush();
  return pieces;
}

/**
 * Place pieces at given offsets, each at its own or directly under the piece
 * above, and moved up only as far as `bottom` requires — the rule
 * `placePieces` applies to estimated offsets.
 */
export function placeAt(targets: number[], heights: number[], gap = 0, bottom = Infinity): number[] {
  const tops: number[] = [];
  let free = 0;
  targets.forEach((target, index) => {
    const top = Math.max(target, free);
    tops.push(top);
    free = top + (heights[index] ?? 0) + gap;
  });
  let limit = bottom;
  for (let index = tops.length - 1; index >= 0; index--) {
    tops[index] = Math.max(0, Math.min(tops[index], limit - (heights[index] ?? 0)));
    limit = tops[index] - gap;
  }
  return tops;
}

/**
 * The words being said at `seconds`, as indexes into a list sorted by start.
 *
 * A word counts until it ends; the two sides can overlap, so there may be
 * more than one. Only a bounded stretch before the moment is examined: no
 * word is that long.
 */
export function wordsAt(words: TimedWord[], seconds: number, lookBack = 40): number[] {
  let low = 0;
  let high = words.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (words[mid].s <= seconds) low = mid + 1;
    else high = mid;
  }
  const found: number[] = [];
  for (let index = low - 1; index >= 0 && index >= low - lookBack; index--) {
    if (words[index].e > seconds) found.push(index);
  }
  return found.reverse();
}

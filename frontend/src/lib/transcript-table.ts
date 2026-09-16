/**
 * Laying a transcript out as a table: one row per stretch of conversation,
 * one column per side.
 *
 * The two sides are recorded on separate tracks with one clock, so an
 * interruption is simply two lines whose times overlap. The chat view puts
 * them one after the other and the overlap is lost; here they share a row,
 * side by side, which is the whole point of the table.
 *
 * A row is a run of lines that overlap one another in time — directly or
 * through a line in between. A line that starts after everything before it
 * has ended opens a new row. A line spoken by both at once (`both`) fills both
 * columns of its row.
 *
 * Pure on purpose: no React, no Tauri, so it can be tested with plain node.
 */

export type TableSide = 'host' | 'client' | 'both';

export interface TableLine {
  id: string;
  /** Seconds from the start of the recording. */
  start: number;
  /** Seconds; a line with no known end is treated as a point in time. */
  end?: number;
  text: string;
  speaker?: string;
  confidence?: number;
}

export interface TableRow {
  /** Stable across renders: the id of the row's first line. */
  key: string;
  start: number;
  end: number;
  /** In time order. A `both` line is in both lists. */
  host: TableLine[];
  client: TableLine[];
  /** Every id in the row, for finding the row the recording is at. */
  ids: string[];
}

function endOf(line: TableLine): number {
  return Math.max(line.start, line.end ?? line.start);
}

/**
 * Group lines into rows.
 *
 * Lines are sorted by start first: the two tracks are transcribed separately
 * and arrive interleaved only approximately.
 */
export function buildTableRows(
  lines: TableLine[],
  sideOf: (speaker?: string) => TableSide,
): TableRow[] {
  const sorted = [...lines].sort((a, b) => a.start - b.start || endOf(a) - endOf(b));
  const rows: TableRow[] = [];
  let current: TableRow | null = null;

  for (const line of sorted) {
    const lineEnd = endOf(line);
    // Strictly before: a line that starts exactly when the last one ended is
    // the next turn, not an interruption.
    if (!current || line.start >= current.end) {
      current = {
        key: line.id,
        start: line.start,
        end: lineEnd,
        host: [],
        client: [],
        ids: [],
      };
      rows.push(current);
    } else {
      current.end = Math.max(current.end, lineEnd);
    }

    const side = sideOf(line.speaker);
    if (side === 'host' || side === 'both') current.host.push(line);
    if (side === 'client' || side === 'both') current.client.push(line);
    current.ids.push(line.id);
  }
  return rows;
}

/** Index of the row holding `id`, or -1. */
export function rowIndexOf(rows: TableRow[], id: string | null | undefined): number {
  if (!id) return -1;
  return rows.findIndex((row) => row.ids.includes(id));
}

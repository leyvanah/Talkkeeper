/**
 * Sizes and visibility of the resizable panels, and how they are remembered.
 *
 * Everything here is a per-viewer convenience: it lives in localStorage and a
 * missing or unreadable value falls back to the default layout.
 */

export const SIDEBAR_COLLAPSED_WIDTH = 64;
export const SIDEBAR_MIN_WIDTH = 200;
export const SIDEBAR_MAX_WIDTH = 480;
export const SIDEBAR_DEFAULT_WIDTH = 256;

/** Dragged nearer the window's edge than this, the sidebar collapses. */
export const SIDEBAR_COLLAPSE_AT = 150;

/** Share of the meeting page the transcript takes when both columns show. */
export const SPLIT_MIN = 0.25;
export const SPLIT_MAX = 0.75;
export const SPLIT_DEFAULT = 0.47;

/** Neither column of the meeting page is made narrower than this. */
export const PANE_MIN_WIDTH = 280;

/** A column dragged narrower than this is hidden instead. */
export const PANE_COLLAPSE_AT = 170;

/** Which of the meeting page's two columns are showing. */
export type MeetingPanes = 'both' | 'transcript' | 'summary';

export interface PanelLayout {
  sidebarWidth: number;
  sidebarCollapsed: boolean;
  /** The transcript's share of the width when both columns show. */
  split: number;
  panes: MeetingPanes;
}

export const DEFAULT_LAYOUT: PanelLayout = {
  sidebarWidth: SIDEBAR_DEFAULT_WIDTH,
  sidebarCollapsed: false,
  split: SPLIT_DEFAULT,
  panes: 'both',
};

export const LAYOUT_STORAGE_KEY = 'panel_layout';

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function clampSidebarWidth(width: number): number {
  return Math.round(clamp(width, SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH));
}

export function clampSplit(split: number): number {
  return clamp(split, SPLIT_MIN, SPLIT_MAX);
}

/** How much room the sidebar takes from the page. */
export function sidebarOffset(layout: Pick<PanelLayout, 'sidebarWidth' | 'sidebarCollapsed'>): number {
  return layout.sidebarCollapsed ? SIDEBAR_COLLAPSED_WIDTH : layout.sidebarWidth;
}

/**
 * The transcript's share for a pointer at `x` in a row that starts at `left`
 * and is `width` wide. Neither column goes below its minimum width, unless
 * the row is too narrow to hold two of them.
 */
export function splitAt(x: number, left: number, width: number): number {
  if (!(width > 0)) return SPLIT_DEFAULT;
  const floor = PANE_MIN_WIDTH / width;
  if (floor >= 0.5) return 0.5;
  return clamp((x - left) / width, Math.max(SPLIT_MIN, floor), Math.min(SPLIT_MAX, 1 - floor));
}

/**
 * Where the sidebar lands when its edge is dragged to `x`: pulled in towards
 * the window's edge it collapses to the icons, and dragged back out it opens
 * again at the width the pointer is at.
 */
export function sidebarFromDrag(x: number): { collapsed: boolean; width: number } {
  return x < SIDEBAR_COLLAPSE_AT
    ? { collapsed: true, width: SIDEBAR_MIN_WIDTH }
    : { collapsed: false, width: clampSidebarWidth(x) };
}

/**
 * Where the meeting page's columns land when the divider is dragged to `x`:
 * a column pulled in far enough is hidden, leaving the other one whole.
 */
export function panesFromDrag(
  x: number,
  left: number,
  width: number
): { panes: MeetingPanes; split: number } {
  const split = splitAt(x, left, width);
  if (x - left < PANE_COLLAPSE_AT) return { panes: 'summary', split };
  if (left + width - x < PANE_COLLAPSE_AT) return { panes: 'transcript', split };
  return { panes: 'both', split };
}

/**
 * Hide one column, or bring it back. Hiding the only column showing brings
 * the other one back instead of leaving the page empty.
 */
export function togglePane(panes: MeetingPanes, pane: 'transcript' | 'summary'): MeetingPanes {
  if (panes !== 'both') return 'both';
  return pane === 'transcript' ? 'summary' : 'transcript';
}

export function isPaneShown(panes: MeetingPanes, pane: 'transcript' | 'summary'): boolean {
  return panes === 'both' || panes === pane;
}

/** A stored layout, with anything missing or out of range replaced. */
export function parseLayout(raw: string | null | undefined): PanelLayout {
  if (!raw) return { ...DEFAULT_LAYOUT };
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return { ...DEFAULT_LAYOUT };
  }
  if (!value || typeof value !== 'object') return { ...DEFAULT_LAYOUT };
  const v = value as Record<string, unknown>;
  const finite = (n: unknown): n is number => typeof n === 'number' && Number.isFinite(n);
  return {
    sidebarWidth: finite(v.sidebarWidth) ? clampSidebarWidth(v.sidebarWidth) : DEFAULT_LAYOUT.sidebarWidth,
    sidebarCollapsed: typeof v.sidebarCollapsed === 'boolean' ? v.sidebarCollapsed : DEFAULT_LAYOUT.sidebarCollapsed,
    split: finite(v.split) ? clampSplit(v.split) : DEFAULT_LAYOUT.split,
    panes: v.panes === 'transcript' || v.panes === 'summary' || v.panes === 'both' ? v.panes : DEFAULT_LAYOUT.panes,
  };
}

export function loadLayout(): PanelLayout {
  try {
    return parseLayout(window.localStorage.getItem(LAYOUT_STORAGE_KEY));
  } catch {
    return { ...DEFAULT_LAYOUT };
  }
}

export function saveLayout(layout: PanelLayout): void {
  try {
    window.localStorage.setItem(LAYOUT_STORAGE_KEY, JSON.stringify(layout));
  } catch {
    // Storage can be unavailable; the layout then lasts until the window closes.
  }
}

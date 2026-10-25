/**
 * What `api_get_summary` says about a meeting's summary, as the meeting page
 * needs it: is there a summary to show, and did the last attempt fail?
 *
 * A failed attempt with no earlier summary to fall back on comes back as
 * `status: "failed"` and no data. The page used to read that as a summary -
 * `data || {}` - and showed an empty one ("no summary text", "no action
 * items"), with "Regenerate" and no word of the failure (tasks#41).
 */

export interface SummaryResponseLike {
  status?: string | null;
  data?: unknown;
  error?: string | null;
}

export type SummaryOnRecord =
  /** Nothing to show: never generated, still running, or cancelled. */
  | { kind: 'none' }
  /** Nothing to show, and the last attempt failed; `error` is the backend's text. */
  | { kind: 'failed'; error: string | null }
  /** A summary to show. A failed or cancelled regeneration keeps the earlier one. */
  | { kind: 'present'; data: unknown };

/** Whether the stored result holds anything at all. */
export function hasSummaryContent(data: unknown): boolean {
  if (data === null || data === undefined) return false;
  if (typeof data === 'string') {
    const text = data.trim();
    if (!text) return false;
    try {
      return hasSummaryContent(JSON.parse(text));
    } catch {
      return true;
    }
  }
  if (Array.isArray(data)) return data.length > 0;
  if (typeof data === 'object') return Object.keys(data as object).length > 0;
  return true;
}

export function readSummaryResponse(response: SummaryResponseLike | null | undefined): SummaryOnRecord {
  if (!response) return { kind: 'none' };
  const status = (response.status ?? '').toLowerCase();
  if (status !== 'idle' && hasSummaryContent(response.data)) {
    return { kind: 'present', data: response.data };
  }
  if (status === 'failed') return { kind: 'failed', error: response.error ?? null };
  // `error` is the request itself failing (no such meeting), not an attempt.
  return { kind: 'none' };
}

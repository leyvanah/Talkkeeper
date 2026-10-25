"use client";

/**
 * The summary tab of the meeting page's right column: one scrolling column of
 * the AI summary, action items and key topics. Questions about the meeting
 * have a tab of their own (MeetingChat).
 *
 * The summary is sorted into sections in `lib/summary-buckets`. Action items
 * and key topics are shown only when the summary has those sections, and
 * read-only: there is no editor to keep a change in. Owners and due dates on
 * action items are recovered heuristically from the item text.
 */

import { useMemo } from 'react';
import { useTranslations } from 'next-intl';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { modelMarkdownComponents } from '@/lib/model-markdown';
import { Calendar, Clock, Dot, User } from 'lucide-react';
import { Summary, Transcript } from '@/types';
import { isEmptySectionPlaceholder } from '@/lib/summary-placeholder';
import { summaryBuckets } from '@/lib/summary-buckets';

const OWNER_CHIP = [
  'bg-purple-500/15 text-purple-300',
  'bg-blue-500/15 text-blue-300',
  'bg-teal-500/15 text-teal-300',
  'bg-amber-500/15 text-amber-300',
  'bg-pink-500/15 text-pink-300',
  'bg-indigo-500/15 text-indigo-300',
];

function ownerChip(name: string): string {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return OWNER_CHIP[h % OWNER_CHIP.length];
}

const MONTHS = '(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*';

interface ParsedAction {
  owner: string | null;
  due: string | null;
  timestamp: string | null;
  text: string;
}

function parseAction(raw: string): ParsedAction {
  let text = raw.replace(/^[\s\-*•]+/, '').trim();
  let owner: string | null = null;

  let m = text.match(/^@([A-Za-z][\w.'-]*)\s+(.*)$/);
  if (m) { owner = m[1]; text = m[2].trim(); }
  if (!owner) { m = text.match(/^\[([^\]]{1,40})\]\s*[:\-–—]?\s*(.*)$/); if (m) { owner = m[1].trim(); text = m[2].trim(); } }
  if (!owner) { m = text.match(/^([A-Z][a-zA-Z.'-]+(?:\s+[A-Z][a-zA-Z.'-]+){0,2})\s*[:\-–—]\s+(.+)$/); if (m) { owner = m[1].trim(); text = m[2].trim(); } }
  if (!owner) {
    m = text.match(/\b(?:owner|assigned to|assignee|owned by)\s*[:\-]?\s*([A-Z][\w.'-]+(?:\s+[A-Z][\w.'-]+)?)/i);
    if (m) { owner = m[1].trim(); text = text.replace(m[0], '').trim(); }
  }

  let date: string | null = null;
  let d = text.match(/\b(\d{4}-\d{2}-\d{2})\b/);
  if (d) date = d[1];
  if (!date) { d = text.match(new RegExp(`\\b(${MONTHS}\\.?\\s+\\d{1,2}(?:st|nd|rd|th)?)\\b`, 'i')); if (d) date = d[1]; }
  if (!date) { d = text.match(/\b(tomorrow|next week|end of week|EOD|Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday)\b/i); if (d) date = d[1]; }
  if (date && d) {
    text = text.replace(d[0], '').replace(/\b(by|due|on|before)\s*$/i, '').replace(/[\(\[]\s*[\)\]]?\s*$/, '').replace(/[,\-–—]\s*$/, '').trim();
  }

  return { owner, due: date, timestamp: null, text: text.replace(/\s{2,}/g, ' ').trim() };
}

// ---- Markdown-table action items -------------------------------------------
// Models often emit action items as a GFM table (| Owner | Task | Due | … |).
// Splitting that by line turns the header row and the `| --- |` separator into
// bogus "action items" and crams every column into one line. These helpers
// parse the table into structured items instead, dropping header/separator.

function splitTableRow(row: string): string[] {
  let s = row.trim();
  if (s.startsWith('|')) s = s.slice(1);
  if (s.endsWith('|')) s = s.slice(0, -1);
  return s.split('|').map((c) => c.trim());
}

function isSeparatorRow(cells: string[]): boolean {
  return cells.every((c) => c === '' || /^:?-{2,}:?$/.test(c.replace(/\s/g, '')));
}

function stripCell(s: string): string {
  return s
    .replace(/\*\*/g, '')
    .replace(/^["'“”\s]+|["'“”\s]+$/g, '')
    .trim();
}

function parseActionTable(rows: string[]): ParsedAction[] {
  const grid = rows.map(splitTableRow);
  let h = 0;
  while (h < grid.length && isSeparatorRow(grid[h])) h++;
  const header = (grid[h] ?? []).map((c) => stripCell(c).toLowerCase());
  const col = (re: RegExp) => header.findIndex((c) => re.test(c));
  const ownerIdx = col(/owner|assignee|assigned|who|responsible|ответствен|исполнител|кто/);
  const taskIdx = col(/task|action|item|descr|to-?do|deliverable|next|задач|действи|описан|шаг/);
  const dueIdx = col(/due|date|when|deadline|timeline|срок|дата|когда/);
  const timeIdx = col(/time.?stamp|timestamp/);

  const items: ParsedAction[] = [];
  for (let i = h + 1; i < grid.length; i++) {
    const cells = grid[i];
    if (isSeparatorRow(cells) || cells.every((c) => c === '')) continue;
    const owner = ownerIdx >= 0 ? stripCell(cells[ownerIdx] ?? '') : '';
    const due = dueIdx >= 0 ? stripCell(cells[dueIdx] ?? '') : '';
    const time = timeIdx >= 0 ? stripCell(cells[timeIdx] ?? '') : '';
    let task = taskIdx >= 0 ? stripCell(cells[taskIdx] ?? '') : '';
    if (!task) {
      // No recognizable task column: fall back to the longest remaining cell.
      task =
        cells
          .map(stripCell)
          .filter((c, idx) => c && idx !== ownerIdx && idx !== dueIdx && idx !== timeIdx)
          .sort((a, b) => b.length - a.length)[0] ?? '';
    }
    // A table row standing in for "no action items": the placeholder in the
    // task column, and "not specified" in the others.
    if (!task || isEmptySectionPlaceholder(task)) continue;
    items.push({
      owner: owner || null,
      due: due || null,
      timestamp: time ? time.replace(/^[[(]+|[\])]+$/g, '').trim() : null,
      text: task,
    });
  }
  return items;
}

/** Turn the raw action bucket (a markdown table and/or plain list) into
 *  structured items. Table header/separator rows are dropped, not shown. */
function parseActionItems(lines: string[]): ParsedAction[] {
  const table = lines.filter((l) => l.trim().startsWith('|'));
  const plain = lines.filter((l) => !l.trim().startsWith('|'));
  const items: ParsedAction[] = [];
  if (table.length >= 2) items.push(...parseActionTable(table));
  for (const line of plain) {
    const t = line.trim();
    if (!t) continue;
    const a = parseAction(t);
    if (a.text) items.push(a);
  }
  return items;
}

function topicLabel(raw: string): string {
  const t = raw.replace(/^[\s\-*•]+/, '').trim();
  const colon = t.indexOf(':');
  if (colon > 0 && colon <= 40) return t.slice(0, colon).trim();
  return t.length > 40 ? t.slice(0, 38).trim() + '…' : t;
}


/** Why the last attempt to create a summary failed, for a meeting that has none. */
export interface SummaryFailure {
  /** A sentence the owner can act on, when the cause is a known one. */
  explained: string | null;
  /** The backend's own text, for the tooltip. */
  raw: string | null;
}

interface InsightTabsProps {
  aiSummary: Summary | null;
  transcripts: Transcript[];
  generating?: boolean;
  failure?: SummaryFailure | null;
}

function Spinner({ label }: { label: string }) {
  return (
    <div className="flex items-center gap-3 py-2 text-sm text-[var(--af-text-2)]">
      <span className="h-4 w-4 animate-spin rounded-full border-2 border-[var(--af-accent)] border-t-transparent" />
      {label}
    </div>
  );
}

export function InsightTabs({
  aiSummary,
  transcripts,
  generating = false,
  failure = null,
}: InsightTabsProps) {
  const t = useTranslations('meetingDetails');

  const buckets = useMemo(() => summaryBuckets(aiSummary, isEmptySectionPlaceholder), [aiSummary]);
  const actions = useMemo(() => parseActionItems(buckets.items.actions), [buckets.items.actions]);
  const summaryText = useMemo(() => buckets.items.summary.join('\n\n'), [buckets.items.summary]);
  const hasSummary = !!aiSummary;
  const showActions = hasSummary && !generating && buckets.present.has('actions');
  const showTopics = hasSummary && !generating && buckets.present.has('topics');

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1 space-y-8 overflow-y-auto px-6 pb-6 pt-3">
        {/* AI Summary */}
        <section>
          <h3 className="mb-3 text-base font-semibold text-[var(--af-text)]">{t('aiSummaryHeading')}</h3>
          {generating ? (
            <Spinner label={t('generatingSummary')} />
          ) : !hasSummary ? (
            <div className="py-2">
              <p className="mb-1 text-sm text-[var(--af-text-2)]">{t('noSummaryYet')}</p>
              {failure && (
                <p
                  className="mb-1 text-sm text-red-600 dark:text-red-400"
                  title={failure.raw ?? undefined}
                  role="status"
                >
                  {t('summaryLastAttemptFailed')}
                  {failure.explained ? ` ${failure.explained}` : ''}
                </p>
              )}
              <p className="text-xs text-[var(--af-text-3)]">
                {transcripts.length > 0
                  ? t('summaryHintWithTranscript')
                  : t('summaryHintNoTranscript')}
              </p>
            </div>
          ) : summaryText ? (
            <div className="prose prose-sm max-w-none leading-relaxed text-[var(--af-text-2)] dark:prose-invert prose-strong:text-[var(--af-text)] prose-p:my-2">
              <ReactMarkdown remarkPlugins={[remarkGfm]} components={modelMarkdownComponents}>{summaryText}</ReactMarkdown>
            </div>
          ) : (
            <p className="text-sm text-[var(--af-text-3)]">{t('noSummaryText')}</p>
          )}
        </section>

        {/* Action Items: only for a summary whose template has them */}
        {showActions && (
          <section>
            <div className="mb-1 flex items-center gap-2">
              <h3 className="text-base font-semibold text-[var(--af-text)]">{t('actionItemsHeading')}</h3>
              {actions.length > 0 && <span className="text-sm font-medium text-[var(--af-text-3)]">{actions.length}</span>}
            </div>
            {actions.length === 0 ? (
              <div className="py-2 text-sm text-[var(--af-text-3)]">{t('noActionItems')}</div>
            ) : (
              <ul>
                {actions.map((a, i) => (
                  <li key={i} className="flex items-start gap-2 border-b border-[var(--af-border)] py-3 last:border-0">
                    <Dot size={18} className="mt-0.5 shrink-0 text-[var(--af-text-3)]" aria-hidden />
                    <div className="min-w-0 flex-1">
                      <p className="text-sm leading-relaxed text-[var(--af-text)]">{a.text}</p>
                      {(a.owner || a.due || a.timestamp) && (
                        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                          {a.owner && (
                            <span className={`inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-xs font-medium ${ownerChip(a.owner)}`}>
                              <User size={11} />
                              {a.owner}
                            </span>
                          )}
                          {a.due && (
                            <span className="inline-flex items-center gap-1 rounded-md bg-[var(--af-panel-2)] px-2 py-0.5 text-xs text-[var(--af-text-2)]">
                              <Calendar size={11} />
                              {a.due}
                            </span>
                          )}
                          {a.timestamp && (
                            <span className="inline-flex items-center gap-1 rounded-md border border-[var(--af-border)] px-2 py-0.5 font-mono text-[11px] text-[var(--af-text-3)]">
                              <Clock size={11} />
                              {a.timestamp}
                            </span>
                          )}
                        </div>
                      )}
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </section>
        )}

        {/* Key Topics: only for a summary whose template has them */}
        {showTopics && (
          <section>
            <h3 className="mb-3 text-base font-semibold text-[var(--af-text)]">{t('keyTopicsHeading')}</h3>
            {buckets.items.topics.length === 0 ? (
              <div className="py-2 text-sm text-[var(--af-text-3)]">{t('noTopics')}</div>
            ) : (
              <div className="flex flex-wrap gap-2">
                {buckets.items.topics.map((topic, i) => (
                  <span
                    key={i}
                    title={topic}
                    className="rounded-md bg-[var(--af-panel-2)] px-2.5 py-1 text-sm text-[var(--af-text-2)]"
                  >
                    {topicLabel(topic)}
                  </span>
                ))}
              </div>
            )}
          </section>
        )}
      </div>
    </div>
  );
}

export default InsightTabs;

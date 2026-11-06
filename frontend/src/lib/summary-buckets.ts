/**
 * A stored summary, sorted into what the meeting page shows: the summary
 * itself, action items, key topics, and the rest.
 *
 * The stored summary can arrive as markdown, BlockNote JSON, or a legacy section
 * map; all three are flattened into the same buckets so the panel renders
 * regardless of which the backend/model produced.
 *
 * `present` says which sections the summary actually has. The page shows
 * "Action items" and "Key topics" only for a summary whose template asked for
 * them; before, every summary got both, as "none found" (tasks#89).
 */

/** The model's "nothing here" line (`isEmptySectionPlaceholder`). Taken as a
 * parameter so the node tests can load this file without a bundler. */
export type IsPlaceholder = (text: string) => boolean;

export type Bucket = 'summary' | 'actions' | 'topics' | 'insights';

export interface SummaryBuckets {
  items: Record<Bucket, string[]>;
  /** Sections the summary has, with items or with only a "nothing here" line. */
  present: Set<Bucket>;
}

// Headings come from the model, so they follow the summary language, not the UI
// language. Both English and Russian wordings are matched or a Russian summary
// would fall through to `insights` and the panel would render one flat list.
const RE = {
  actions: /(action|task|to-?do|next[ -]?step|follow[ -]?up|deliverable|задач|действи|шаг|поручен)/i,
  topics: /(topic|theme|discuss|agenda|subject|тем[аы]|обсужд|повестк)/i,
  summary: /(summary|overview|abstract|tl;?dr|key[ -]?point|highlight|recap|саммари|резюме|обзор|кратк|основн|главн)/i,
  insights: /(insight|decision|risk|take[ -]?away|conclusion|outcome|blocker|learn|решени|риск|вывод|итог|наблюден)/i,
};

function classify(h: string): Bucket {
  if (RE.actions.test(h)) return 'actions';
  if (RE.topics.test(h)) return 'topics';
  if (RE.summary.test(h)) return 'summary';
  if (RE.insights.test(h)) return 'insights';
  return 'insights';
}

function inlineText(node: any): string {
  if (node == null) return '';
  if (typeof node === 'string') return node;
  if (Array.isArray(node)) return node.map(inlineText).join('');
  if (typeof node === 'object') {
    if (typeof node.text === 'string') return node.text;
    if (node.content) return inlineText(node.content);
  }
  return '';
}

function blocksToMarkdown(blocks: any[]): string {
  const out: string[] = [];
  const walk = (list: any[]) => {
    for (const b of list || []) {
      const text = inlineText(b?.content).trim();
      const type = b?.type;
      if (type === 'heading') out.push(`## ${text}`);
      else if (type === 'bulletListItem' || type === 'numberedListItem' || type === 'checkListItem') out.push(`- ${text}`);
      else if (text) out.push(text);
      if (Array.isArray(b?.children) && b.children.length) walk(b.children);
    }
  };
  walk(blocks);
  return out.join('\n');
}

function headingOf(line: string): string | null {
  let m = line.match(/^#{1,6}\s+(.*)$/); if (m) return m[1].replace(/[:*]+$/, '').trim();
  m = line.match(/^\*\*(.+?)\*\*:?\s*$/); if (m) return m[1].trim();
  m = line.match(/^([A-ZА-ЯЁ][A-Za-zА-Яа-яЁё /&]{2,40}):\s*$/); if (m) return m[1].trim();
  return null;
}

function emptyBuckets(): SummaryBuckets {
  return { items: { summary: [], actions: [], topics: [], insights: [] }, present: new Set() };
}

function parseMarkdownBuckets(md: string, isEmptySectionPlaceholder: IsPlaceholder): SummaryBuckets {
  const out = emptyBuckets();
  let current: Bucket = 'summary';
  for (const raw of md.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || /^[-=*_]{3,}$/.test(line)) continue;
    const h = headingOf(line);
    if (h) {
      current = classify(h);
      out.present.add(current);
      continue;
    }
    const item = line.replace(/^[-*+]\s+/, '').replace(/^\d+[.)]\s+/, '').replace(/^#+\s*/, '').trim();
    // An empty section holds the model's "nothing here" line, not an item.
    if (item && !isEmptySectionPlaceholder(item)) {
      out.items[current].push(item);
      out.present.add(current);
    }
  }
  return out;
}

function bucketizeSections(summary: any, isEmptySectionPlaceholder: IsPlaceholder): SummaryBuckets {
  const out = emptyBuckets();
  const leftovers: string[] = [];
  const skip = new Set(['markdown', 'summary_json', '_section_order', 'MeetingName']);
  for (const [key, section] of Object.entries(summary)) {
    if (skip.has(key) || !section || !Array.isArray((section as any).blocks)) continue;
    const items = (section as any).blocks
      .map((b: any) => (b?.content ?? '').trim())
      .filter((text: string) => text && !isEmptySectionPlaceholder(text));
    const label = `${key} ${(section as any).title ?? ''}`;
    const bucket: Bucket | null = RE.actions.test(label)
      ? 'actions'
      : RE.topics.test(label)
        ? 'topics'
        : RE.summary.test(label)
          ? 'summary'
          : RE.insights.test(label)
            ? 'insights'
            : null;
    if (bucket) out.present.add(bucket);
    if (items.length === 0) continue;
    if (bucket) out.items[bucket].push(...items);
    else leftovers.push(...items);
  }
  out.items.insights.push(...leftovers);
  if (out.items.summary.length === 0) {
    out.items.summary = (leftovers.length ? leftovers : out.items.insights).slice(0, 4);
  }
  return out;
}

export function summaryBuckets(aiSummary: any, isPlaceholder: IsPlaceholder): SummaryBuckets {
  if (!aiSummary) return emptyBuckets();
  if (typeof aiSummary === 'string') return parseMarkdownBuckets(aiSummary, isPlaceholder);
  if (typeof aiSummary.markdown === 'string') return parseMarkdownBuckets(aiSummary.markdown, isPlaceholder);
  if (Array.isArray(aiSummary.summary_json)) return parseMarkdownBuckets(blocksToMarkdown(aiSummary.summary_json), isPlaceholder);
  return bucketizeSections(aiSummary, isPlaceholder);
}

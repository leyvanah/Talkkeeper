/**
 * The meeting export's Markdown, read as the blocks a page lays out: headings,
 * bullets, paragraphs, rules and tables. Only what the export itself writes is
 * understood — summaries are Markdown from the model, and the transcript is a
 * line per utterance.
 */

export type MarkdownBlock =
  | { kind: 'blank' }
  | { kind: 'rule' }
  | { kind: 'heading'; level: 1 | 2 | 3 | 4; text: string }
  | { kind: 'bullet'; text: string }
  | { kind: 'paragraph'; text: string }
  | { kind: 'table'; header: string[]; rows: string[][] };

/** Plain text of a line: emphasis markers and inline code ticks dropped. */
export function inlineText(text: string): string {
  return text
    .replace(/\*\*([^*]+)\*\*/g, '$1')
    .replace(/\*\*/g, '')
    .replace(/(^|[^\w*])\*([^*\s][^*]*)\*(?=[^\w*]|$)/g, '$1$2')
    .replace(/`([^`]+)`/g, '$1')
    .trim();
}

const isTableRow = (line: string) => /^\s*\|.*\|\s*$/.test(line);
const isSeparatorRow = (line: string) => /^\s*\|(\s*:?-{3,}:?\s*\|)+\s*$/.test(line);

/** The cells of `| a | b |`, as plain text. */
export function tableCells(line: string): string[] {
  const inner = line.trim().replace(/^\|/, '').replace(/\|$/, '');
  return inner.split('|').map((cell) => inlineText(cell));
}

export function markdownBlocks(markdown: string): MarkdownBlock[] {
  const lines = markdown.replace(/\r\n/g, '\n').split('\n');
  const blocks: MarkdownBlock[] = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].replace(/\s+$/g, '');

    // A table is a header row followed by a separator row.
    if (isTableRow(line) && i + 1 < lines.length && isSeparatorRow(lines[i + 1])) {
      const header = tableCells(line);
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && isTableRow(lines[i])) {
        const cells = tableCells(lines[i]);
        // Rows with too few or too many cells are evened out to the header.
        rows.push(header.map((_, column) => cells[column] ?? ''));
        i += 1;
      }
      i -= 1;
      blocks.push({ kind: 'table', header, rows });
      continue;
    }

    if (line.trim() === '') { blocks.push({ kind: 'blank' }); continue; }
    if (/^\s*(-{3,}|\*{3,})\s*$/.test(line)) { blocks.push({ kind: 'rule' }); continue; }

    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      const level = Math.min(heading[1].length, 4) as 1 | 2 | 3 | 4;
      blocks.push({ kind: 'heading', level, text: inlineText(heading[2]) });
      continue;
    }

    const bullet = line.match(/^\s*(?:[-*+]|\d+[.)])\s+(.*)$/);
    if (bullet) { blocks.push({ kind: 'bullet', text: inlineText(bullet[1]) }); continue; }

    blocks.push({ kind: 'paragraph', text: inlineText(line) });
  }
  return blocks;
}

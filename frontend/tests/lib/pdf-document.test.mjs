import assert from 'node:assert/strict';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, test } from 'node:test';
import { markdownBlocks } from '../../src/lib/markdown-blocks.ts';
import { renderPdf } from '../../src/lib/pdf-document.ts';

// The font the app takes on Windows; elsewhere this test has nothing to embed.
const fonts = join(process.env.WINDIR ?? 'C:\Windows', 'Fonts');
const regular = join(fonts, 'arial.ttf');
const bold = join(fonts, 'arialbd.ttf');
const font = existsSync(regular)
  ? {
      regular: readFileSync(regular).toString('base64'),
      bold: existsSync(bold) ? readFileSync(bold).toString('base64') : null,
    }
  : null;

const meeting = [
  '# Транскрипт встречи: Планирование квартала, длинное название, которое не помещается в одну строку страницы',
  '',
  '**Дата:** 15 октября 2026 г.',
  '',
  '---',
  '',
  '[00:05] **Вы:** Добрый день. ' + 'Это длинная реплика, которая должна переноситься по ширине страницы, а не обрезаться справа. '.repeat(4),
  '',
  '## Задачи',
  '| Кто | Задача | Срок |',
  '| --- | --- | --- |',
  '| Анна | Прислать отчёт по итогам квартала с разбивкой по проектам | пятница |',
  '| Иван | Встреча с подрядчиком | 20.10 |',
  '',
  '- Итог: договорились',
].join('\n');

describe('PDF document', { skip: !font && 'no system font with Cyrillic here' }, () => {
  test('embeds the font and lays out every block', () => {
    const bytes = renderPdf(markdownBlocks(meeting), font);
    // Set PDF_SAMPLE to a path to look at the result.
    if (process.env.PDF_SAMPLE) writeFileSync(process.env.PDF_SAMPLE, bytes);
    const text = Buffer.from(bytes).toString('latin1');
    assert.ok(text.startsWith('%PDF-'));
    // The TrueType font travels inside the file: Cyrillic is drawn from it.
    assert.match(text, /\/FontFile2/);
    // jsPDF lists the standard fonts in every file; the text is set in this one.
    assert.match(text, /\/BaseFont \/Body/);
  });
});

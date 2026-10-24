/**
 * The meeting export as a PDF page by page: headings, bullets, paragraphs and
 * tables, set in a font with Cyrillic and wrapped to the page.
 *
 * Kept apart from the export pipeline, with nothing but jsPDF to import, so
 * the tests can build a real file with it.
 */

import { jsPDF } from 'jspdf';
import type { MarkdownBlock } from './markdown-blocks';

/** The font the PDF is set in; see `export_fonts.rs` in the backend. */
export interface PdfFont {
  regular: string;
  bold: string | null;
}

const FONT_FAMILY = 'Body';

/**
 * The standard PDF fonts have no Cyrillic: Russian came out as garbage, spaced
 * out letter by letter, and wrapping went wrong with it. A system font with
 * Cyrillic is embedded instead, read from the system's font folder.
 */
export function renderPdf(blocks: MarkdownBlock[], font: PdfFont): Uint8Array {
  const doc = new jsPDF({ unit: 'pt', format: 'a4' });
  doc.addFileToVFS('Body-Regular.ttf', font.regular);
  doc.addFont('Body-Regular.ttf', FONT_FAMILY, 'normal');
  doc.addFileToVFS('Body-Bold.ttf', font.bold ?? font.regular);
  doc.addFont('Body-Bold.ttf', FONT_FAMILY, 'bold');

  const marginX = 48;
  const marginTop = 56;
  const pageWidth = doc.internal.pageSize.getWidth();
  const pageHeight = doc.internal.pageSize.getHeight();
  const maxWidth = pageWidth - marginX * 2;
  let y = marginTop;

  const ensureSpace = (height: number) => {
    if (y + height > pageHeight - marginTop) {
      doc.addPage();
      y = marginTop;
    }
  };

  const setFont = (size: number, bold: boolean) => {
    doc.setFont(FONT_FAMILY, bold ? 'bold' : 'normal');
    doc.setFontSize(size);
  };

  const writeWrapped = (text: string, size: number, bold: boolean, gapAfter = 4, indent = 0) => {
    setFont(size, bold);
    const wrapped: string[] = doc.splitTextToSize(text, maxWidth - indent);
    const lineHeight = size * 1.3;
    for (const line of wrapped) {
      ensureSpace(lineHeight);
      doc.text(line, marginX + indent, y);
      y += lineHeight;
    }
    y += gapAfter;
  };

  /** A table drawn as cells, each wrapped to its column. */
  const writeTable = (header: string[], rows: string[][]) => {
    const size = 10;
    const pad = 4;
    const lineHeight = size * 1.3;
    // Columns share the width by how much they hold, none narrower than 60pt.
    setFont(size, false);
    const weights = header.map((title, column) => Math.max(
      doc.getTextWidth(title),
      ...rows.map((row) => Math.min(doc.getTextWidth(row[column] ?? ''), maxWidth)),
      1,
    ));
    const total = weights.reduce((sum, weight) => sum + weight, 0);
    let widths = weights.map((weight) => Math.max(60, (weight / total) * maxWidth));
    const scale = maxWidth / widths.reduce((sum, width) => sum + width, 0);
    widths = widths.map((width) => width * scale);

    const drawRow = (cells: string[], bold: boolean) => {
      setFont(size, bold);
      const wrapped = cells.map((cell, column) => doc.splitTextToSize(cell, widths[column] - pad * 2) as string[]);
      const height = Math.max(...wrapped.map((lines) => lines.length), 1) * lineHeight + pad * 2;
      ensureSpace(height);
      // y is a text baseline everywhere else; here it is the row's top edge.
      const top = y - size;
      let x = marginX;
      wrapped.forEach((lines, column) => {
        if (bold) {
          doc.setFillColor(240, 240, 240);
          doc.rect(x, top, widths[column], height, 'FD');
        } else {
          doc.rect(x, top, widths[column], height, 'S');
        }
        lines.forEach((line, index) => {
          doc.text(line, x + pad, top + pad + size + index * lineHeight);
        });
        x += widths[column];
      });
      y += height;
    };

    doc.setDrawColor(190);
    drawRow(header, true);
    for (const row of rows) drawRow(row, false);
    y += 8;
  };

  for (const block of blocks) {
    switch (block.kind) {
      case 'blank':
        y += 6;
        break;
      case 'rule':
        ensureSpace(10);
        doc.setDrawColor(200);
        doc.line(marginX, y, pageWidth - marginX, y);
        y += 12;
        break;
      case 'heading': {
        const size = block.level === 1 ? 20 : block.level === 2 ? 16 : block.level === 3 ? 13 : 12;
        writeWrapped(block.text, size, true, 6);
        break;
      }
      case 'bullet':
        writeWrapped('•  ' + block.text, 11, false, 2, 10);
        break;
      case 'paragraph':
        writeWrapped(block.text, 11, false, 4);
        break;
      case 'table':
        writeTable(block.header, block.rows);
        break;
    }
  }

  return new Uint8Array(doc.output('arraybuffer'));
}

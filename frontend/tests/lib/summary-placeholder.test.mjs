import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import {
  EMPTY_SECTION_MARKER,
  isEmptySectionPlaceholder,
} from '../../src/lib/summary-placeholder.ts';

describe('isEmptySectionPlaceholder', () => {
  test('the marker the prompt asks for, bare or as a list item', () => {
    assert.equal(isEmptySectionPlaceholder(EMPTY_SECTION_MARKER), true);
    assert.equal(isEmptySectionPlaceholder(`- ${EMPTY_SECTION_MARKER}`), true);
    assert.equal(isEmptySectionPlaceholder(`**${EMPTY_SECTION_MARKER}**`), true);
  });

  test('other dashes a model may write instead', () => {
    for (const line of ['-', '–', '--', ' — ']) {
      assert.equal(isEmptySectionPlaceholder(line), true, line);
    }
  });

  test('the sentence older summaries carry', () => {
    assert.equal(isEmptySectionPlaceholder('None noted in this section.'), true);
    assert.equal(isEmptySectionPlaceholder('- *None noted in this section*'), true);
  });

  test('short "nothing" answers, in English and Russian', () => {
    for (const line of ['None', 'N/A', 'n/a.', 'Нет', 'Н/А']) {
      assert.equal(isEmptySectionPlaceholder(line), true, line);
    }
  });

  test('a real item is kept, even one that starts with a dash or mentions none', () => {
    for (const line of [
      '- Send the draft to the team by Friday',
      'Anna: book the room',
      '— Discuss the budget',
      'None of the options suited the owner',
      'Нет возражений по плану',
    ]) {
      assert.equal(isEmptySectionPlaceholder(line), false, line);
    }
  });
});

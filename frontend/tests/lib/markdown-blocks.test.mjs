import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { inlineText, markdownBlocks, tableCells } from '../../src/lib/markdown-blocks.ts';

describe('markdownBlocks', () => {
  test('reads headings, bullets, paragraphs and rules', () => {
    const blocks = markdownBlocks('# Транскрипт встречи\n\n- **Итог:** договорились\n---\n[00:05] **Вы:** Добрый день');
    assert.deepEqual(blocks, [
      { kind: 'heading', level: 1, text: 'Транскрипт встречи' },
      { kind: 'blank' },
      { kind: 'bullet', text: 'Итог: договорились' },
      { kind: 'rule' },
      { kind: 'paragraph', text: '[00:05] Вы: Добрый день' },
    ]);
  });

  test('reads a summary table as a table, not as text', () => {
    const blocks = markdownBlocks(
      '## Задачи\n| Кто | Задача | Срок |\n| --- | --- | :---: |\n| Анна | Прислать **отчёт** | пятница |\n| Иван | Письмо |\n\nДальше',
    );
    assert.deepEqual(blocks, [
      { kind: 'heading', level: 2, text: 'Задачи' },
      {
        kind: 'table',
        header: ['Кто', 'Задача', 'Срок'],
        rows: [
          ['Анна', 'Прислать отчёт', 'пятница'],
          // A short row is evened out to the header.
          ['Иван', 'Письмо', ''],
        ],
      },
      { kind: 'blank' },
      { kind: 'paragraph', text: 'Дальше' },
    ]);
  });

  test('a lone row of pipes without a separator stays text', () => {
    assert.deepEqual(markdownBlocks('| not | a table |'), [
      { kind: 'paragraph', text: '| not | a table |' },
    ]);
  });

  test('numbered items and deep headings', () => {
    assert.deepEqual(markdownBlocks('1. Первое\n###### Мелкий'), [
      { kind: 'bullet', text: 'Первое' },
      { kind: 'heading', level: 4, text: 'Мелкий' },
    ]);
  });
});

describe('inline text', () => {
  test('drops emphasis and code markers', () => {
    assert.equal(inlineText('**Вы:** *важно* и `код`'), 'Вы: важно и код');
  });

  test('cells keep their own text', () => {
    assert.deepEqual(tableCells('| a | **b** |  |'), ['a', 'b', '']);
  });
});

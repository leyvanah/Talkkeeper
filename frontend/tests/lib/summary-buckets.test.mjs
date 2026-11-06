import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { summaryBuckets } from '../../src/lib/summary-buckets.ts';
import { isEmptySectionPlaceholder } from '../../src/lib/summary-placeholder.ts';

const buckets = (summary) => summaryBuckets(summary, isEmptySectionPlaceholder);

describe('summaryBuckets: which sections a summary has', () => {
  test('a template without tasks or topics has neither', () => {
    const { present, items } = buckets({ markdown: '## Summary\nWe talked.\n\n## Decisions\n- Ship it' });
    assert.equal(present.has('actions'), false);
    assert.equal(present.has('topics'), false);
    assert.deepEqual(items.summary, ['We talked.']);
  });

  test('a template with tasks and topics has both, with their items', () => {
    const { present, items } = buckets({
      markdown: '## Summary\nOk.\n\n## Action items\n- Send the notes\n\n## Key topics\n- Budget',
    });
    assert.ok(present.has('actions'));
    assert.ok(present.has('topics'));
    assert.deepEqual(items.actions, ['Send the notes']);
    assert.deepEqual(items.topics, ['Budget']);
  });

  test('a section the model left empty is still a section, with no items', () => {
    const { present, items } = buckets({ markdown: '## Задачи\n- —\n\n## Темы\n- нет' });
    assert.ok(present.has('actions'));
    assert.ok(present.has('topics'));
    assert.deepEqual(items.actions, []);
  });

  test('BlockNote JSON and the legacy section map are read the same way', () => {
    const blocknote = buckets({
      summary_json: [
        { type: 'heading', content: [{ type: 'text', text: 'Summary' }] },
        { type: 'paragraph', content: [{ type: 'text', text: 'Fine.' }] },
      ],
    });
    assert.equal(blocknote.present.has('actions'), false);

    const legacy = buckets({
      ActionItems: { title: 'Action Items', blocks: [{ content: 'Call back' }] },
      Summary: { title: 'Summary', blocks: [{ content: 'Done.' }] },
    });
    assert.ok(legacy.present.has('actions'));
    assert.equal(legacy.present.has('topics'), false);
    assert.deepEqual(legacy.items.actions, ['Call back']);
  });

  test('no summary has no sections', () => {
    assert.equal(buckets(null).present.size, 0);
  });
});

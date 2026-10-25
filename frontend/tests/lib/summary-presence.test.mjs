import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { hasSummaryContent, readSummaryResponse } from '../../src/lib/summary-presence.ts';

const summary = { markdown: '## Summary\nWe agreed on the plan.' };

describe('readSummaryResponse', () => {
  test('nothing generated yet', () => {
    assert.deepEqual(readSummaryResponse({ status: 'idle', data: null }), { kind: 'none' });
  });

  test('a failed attempt with nothing kept is a failure, not an empty summary', () => {
    assert.deepEqual(
      readSummaryResponse({ status: 'failed', data: null, error: 'llama-helper exited with 3' }),
      { kind: 'failed', error: 'llama-helper exited with 3' },
    );
    assert.deepEqual(readSummaryResponse({ status: 'failed', data: {} }), { kind: 'failed', error: null });
  });

  test('a failed regeneration that restored the earlier summary shows it', () => {
    assert.deepEqual(readSummaryResponse({ status: 'failed', data: summary, error: 'x' }), {
      kind: 'present',
      data: summary,
    });
  });

  test('a cancelled attempt with nothing kept is no summary and no failure', () => {
    assert.deepEqual(readSummaryResponse({ status: 'cancelled', data: null }), { kind: 'none' });
  });

  test('a cancelled regeneration that restored the earlier summary shows it', () => {
    assert.equal(readSummaryResponse({ status: 'cancelled', data: summary }).kind, 'present');
  });

  test('still running, with nothing yet', () => {
    assert.deepEqual(readSummaryResponse({ status: 'pending', data: null }), { kind: 'none' });
    assert.deepEqual(readSummaryResponse({ status: 'processing', data: null }), { kind: 'none' });
  });

  test('completed', () => {
    assert.equal(readSummaryResponse({ status: 'completed', data: summary }).kind, 'present');
    assert.equal(readSummaryResponse({ status: 'Completed', data: JSON.stringify(summary) }).kind, 'present');
  });

  test('a completed attempt with an empty result is not a summary', () => {
    assert.deepEqual(readSummaryResponse({ status: 'completed', data: {} }), { kind: 'none' });
  });

  test('the request itself failing is not a failed attempt', () => {
    assert.deepEqual(readSummaryResponse({ status: 'error', data: null, error: 'not found' }), { kind: 'none' });
    assert.deepEqual(readSummaryResponse(null), { kind: 'none' });
  });
});

describe('hasSummaryContent', () => {
  test('empty values and empty JSON are nothing', () => {
    for (const empty of [null, undefined, '', '  ', '{}', 'null', '[]', {}, []]) {
      assert.equal(hasSummaryContent(empty), false, JSON.stringify(empty));
    }
  });

  test('text and filled objects are something', () => {
    for (const filled of ['plain text', '{"markdown":"x"}', { markdown: 'x' }, [1]]) {
      assert.equal(hasSummaryContent(filled), true, JSON.stringify(filled));
    }
  });
});

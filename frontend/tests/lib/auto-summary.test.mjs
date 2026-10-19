import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { autoSummaryInFlight, startAutoSummary, summaryStartKey } from '../../src/lib/auto-summary.ts';

describe('startAutoSummary', () => {
  test('the page and the background runner start one summary between them', async () => {
    let started = 0;
    let accept;
    const generate = () => {
      started += 1;
      return new Promise((resolve) => { accept = resolve; });
    };

    const background = startAutoSummary('m1', generate);
    const page = startAutoSummary('m1', generate);
    accept(true);

    assert.deepEqual(await Promise.all([background, page]), [true, true]);
    assert.equal(started, 1);
    assert.equal(autoSummaryInFlight.has('m1'), false);
  });

  test('a summary the backend refused can be asked for again', async () => {
    let started = 0;
    const refuse = () => { started += 1; return Promise.resolve(false); };

    assert.equal(await startAutoSummary('m2', refuse), false);
    assert.equal(await startAutoSummary('m2', refuse), false);
    assert.equal(started, 2);
  });

  test('a failed start is not left in flight', async () => {
    await assert.rejects(startAutoSummary('m3', () => Promise.reject(new Error('no model'))));
    assert.equal(autoSummaryInFlight.has('m3'), false);
  });

  test('the started mark is per meeting', () => {
    assert.notEqual(summaryStartKey('a'), summaryStartKey('b'));
    assert.equal(summaryStartKey('a'), 'post-call-summary-started:a');
  });
});

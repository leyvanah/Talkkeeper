import assert from 'node:assert/strict';
import { afterEach, describe, mock, test } from 'node:test';
import { runSaveSteps } from '../../src/lib/save-meeting-changes.ts';

const originalConsoleError = console.error;

const ok = () => mock.fn(async () => {});
const failing = () =>
  mock.fn(async () => {
    throw new Error('database is locked');
  });

describe('runSaveSteps', () => {
  afterEach(() => {
    console.error = originalConsoleError;
    mock.restoreAll();
  });

  test('reports nothing failed when every step saves', async () => {
    const title = ok();
    const summary = ok();

    const failed = await runSaveSteps([
      { part: 'title', run: title },
      { part: 'summary', run: summary },
    ]);

    assert.deepEqual(failed, []);
    assert.equal(title.mock.callCount(), 1);
    assert.equal(summary.mock.callCount(), 1);
  });

  test('reports a failed title and still saves the summary', async () => {
    console.error = mock.fn(() => {});
    const summary = ok();

    const failed = await runSaveSteps([
      { part: 'title', run: failing() },
      { part: 'summary', run: summary },
    ]);

    assert.deepEqual(failed, ['title']);
    assert.equal(summary.mock.callCount(), 1);
  });

  test('reports a failed summary', async () => {
    console.error = mock.fn(() => {});

    const failed = await runSaveSteps([
      { part: 'title', run: ok() },
      { part: 'summary', run: failing() },
    ]);

    assert.deepEqual(failed, ['summary']);
  });

  test('reports both when both fail', async () => {
    console.error = mock.fn(() => {});

    const failed = await runSaveSteps([
      { part: 'title', run: failing() },
      { part: 'summary', run: failing() },
    ]);

    assert.deepEqual(failed, ['title', 'summary']);
  });

  test('has nothing to report when there is nothing to save', async () => {
    assert.deepEqual(await runSaveSteps([]), []);
  });
});

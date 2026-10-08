import assert from 'node:assert/strict';
import { describe, mock, test } from 'node:test';
import { UNSUPPORTED, copyTextWith } from '../../src/lib/private-clipboard.ts';

describe('copyTextWith', () => {
  test('uses the private write when there is one', async () => {
    const privately = mock.fn(async () => {});
    const ordinarily = mock.fn(async () => {});

    await copyTextWith('Секретная фраза', privately, ordinarily);

    assert.deepEqual(privately.mock.calls[0].arguments, ['Секретная фраза']);
    assert.equal(ordinarily.mock.callCount(), 0);
  });

  test('falls back to an ordinary copy only where there is no private write', async () => {
    const privately = mock.fn(async () => {
      throw UNSUPPORTED;
    });
    const ordinarily = mock.fn(async () => {});

    await copyTextWith('текст', privately, ordinarily);

    assert.deepEqual(ordinarily.mock.calls[0].arguments, ['текст']);
  });

  test('a failed private write is reported, not retried the ordinary way', async () => {
    const privately = mock.fn(async () => {
      throw 'OpenClipboard failed: access denied';
    });
    const ordinarily = mock.fn(async () => {});

    await assert.rejects(copyTextWith('текст', privately, ordinarily), /OpenClipboard failed/);
    assert.equal(ordinarily.mock.callCount(), 0);
  });
});

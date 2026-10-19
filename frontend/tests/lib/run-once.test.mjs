import assert from 'node:assert/strict';
import { describe, mock, test } from 'node:test';
import { exclusive } from '../../src/lib/run-once.ts';

/** An action that waits until `finish()` is called. */
function pending() {
  let finish;
  const action = mock.fn(() => new Promise((resolve) => { finish = resolve; }));
  return { action, finish: () => finish() };
}

describe('exclusive', () => {
  test('a second call while the first runs does nothing', async () => {
    const { action, finish } = pending();
    const run = exclusive(action);

    const first = run();
    const second = await run();
    finish();

    assert.equal(second, false);
    assert.equal(await first, true);
    assert.equal(action.mock.callCount(), 1);
  });

  test('runs again once the first call is over', async () => {
    const action = mock.fn(async () => {});
    const run = exclusive(action);

    assert.equal(await run(), true);
    assert.equal(await run(), true);
    assert.equal(action.mock.callCount(), 2);
  });

  test('a failed action does not lock the next call out', async () => {
    let fail = true;
    const action = mock.fn(async () => {
      if (fail) throw new Error('database is locked');
    });
    const run = exclusive(action);

    await assert.rejects(run(), /database is locked/);
    fail = false;
    assert.equal(await run(), true);
    assert.equal(action.mock.callCount(), 2);
  });

  test('passes the arguments through', async () => {
    const action = mock.fn(async () => {});
    const run = exclusive(action);

    await run('small', 466);

    assert.deepEqual(action.mock.calls[0].arguments, ['small', 466]);
  });
});

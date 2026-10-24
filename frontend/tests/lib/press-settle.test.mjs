import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { PRESS_SETTLE_MS, settleRemaining } from '../../src/lib/press-settle.ts';

describe('settleRemaining', () => {
  test('a command that returns at once waits out the whole hold', () => {
    assert.equal(settleRemaining(1000, 1000), PRESS_SETTLE_MS);
    assert.equal(settleRemaining(1000, 1002), PRESS_SETTLE_MS - 2);
  });

  test('the second click of a double click lands inside the hold', () => {
    // Observed: the second click came 150-200 ms after the first.
    assert.ok(settleRemaining(1000, 1200) > 0);
  });

  test('a slow command adds no wait', () => {
    assert.equal(settleRemaining(1000, 1000 + PRESS_SETTLE_MS), 0);
    assert.equal(settleRemaining(1000, 1000 + PRESS_SETTLE_MS * 3), 0);
  });

  test('a clock that went backwards waits no longer than the hold', () => {
    assert.equal(settleRemaining(5000, 1000), PRESS_SETTLE_MS);
  });
});

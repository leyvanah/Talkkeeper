import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { ARM_WINDOW_MS, CONFIRM_GRACE_MS, pressTwoStep } from '../../src/lib/two-step-press.ts';

describe('pressTwoStep', () => {
  test('a first press only arms', () => {
    assert.equal(pressTwoStep(null, 1000), 'armed');
  });

  test('a second press within the window confirms', () => {
    assert.equal(pressTwoStep(1000, 1000 + CONFIRM_GRACE_MS), 'confirmed');
    assert.equal(pressTwoStep(1000, 1000 + ARM_WINDOW_MS - 1), 'confirmed');
  });

  test('a double click does not confirm', () => {
    assert.equal(pressTwoStep(1000, 1000), 'ignored');
    assert.equal(pressTwoStep(1000, 1000 + CONFIRM_GRACE_MS - 1), 'ignored');
  });

  test('once the window has passed, a press arms again', () => {
    assert.equal(pressTwoStep(1000, 1000 + ARM_WINDOW_MS), 'armed');
    assert.equal(pressTwoStep(1000, 1000 + ARM_WINDOW_MS * 5), 'armed');
  });

  test('a clock that went backwards re-arms rather than confirms', () => {
    assert.equal(pressTwoStep(5000, 1000), 'armed');
  });
});

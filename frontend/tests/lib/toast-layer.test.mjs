import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { isInsideToaster, keepOpenForToasts } from '../../src/lib/toast-layer.ts';

/** Stands in for a DOM element, inside the toasts or not. */
const element = (inToaster) => ({
  closest: (selector) => (inToaster && selector === '[data-sonner-toaster]' ? {} : null),
});

const outsideEvent = (target) => {
  const event = { target, prevented: false, preventDefault() { this.prevented = true; } };
  return event;
};

describe('toasts over a dialog', () => {
  test('tells a press on a toast from any other target', () => {
    assert.equal(isInsideToaster(element(true)), true);
    assert.equal(isInsideToaster(element(false)), false);
    assert.equal(isInsideToaster(null), false);
    assert.equal(isInsideToaster({}), false);
  });

  test('a press on a toast keeps the dialog open and skips its handler', () => {
    let called = 0;
    const event = outsideEvent(element(true));
    keepOpenForToasts(() => { called += 1; })(event);
    assert.equal(event.prevented, true);
    assert.equal(called, 0);
  });

  test('a press elsewhere goes to the dialog as before', () => {
    let called = 0;
    const event = outsideEvent(element(false));
    keepOpenForToasts(() => { called += 1; })(event);
    assert.equal(event.prevented, false);
    assert.equal(called, 1);
    // No handler given: nothing to call, and the dialog closes as usual.
    const plain = outsideEvent(element(false));
    keepOpenForToasts()(plain);
    assert.equal(plain.prevented, false);
  });
});

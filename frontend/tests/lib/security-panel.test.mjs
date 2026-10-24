import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import {
  IDLE_PANEL,
  outcomeAt,
  secondsLeft,
  securityPanelReducer as step,
} from '../../src/lib/security-panel.ts';

describe('securityPanelReducer', () => {
  test('only the started action is running', () => {
    const state = step(IDLE_PANEL, { type: 'start', action: 'keyBackup' });
    assert.equal(state.running, 'keyBackup');
  });

  test('a second action cannot start while one runs', () => {
    const first = step(IDLE_PANEL, { type: 'start', action: 'recoveryIssue' });
    const second = step(first, { type: 'start', action: 'disable' });
    assert.equal(second.running, 'recoveryIssue');
  });

  test('an error is shown at the action that failed, not on the whole panel', () => {
    let state = step(IDLE_PANEL, { type: 'start', action: 'keyBackup' });
    state = step(state, { type: 'fail', text: 'Wrong password.', now: 0 });
    assert.equal(state.running, null);
    assert.deepEqual(outcomeAt(state, 'keyBackup'), {
      place: 'keyBackup',
      kind: 'error',
      text: 'Wrong password.',
    });
    assert.equal(outcomeAt(state, 'changePassword'), null);
  });

  test('a notice goes to its action unless another place is named', () => {
    let state = step(IDLE_PANEL, { type: 'start', action: 'quick' });
    state = step(state, { type: 'succeed', notice: 'Quick unlock is on.' });
    assert.equal(outcomeAt(state, 'quick')?.kind, 'notice');

    // Removing protection takes its own section away; the notice moves to the top.
    state = step(state, { type: 'start', action: 'disable' });
    state = step(state, { type: 'succeed', notice: 'Protection removed.', place: 'setup' });
    assert.equal(outcomeAt(state, 'disable'), null);
    assert.equal(outcomeAt(state, 'setup')?.text, 'Protection removed.');
  });

  test('starting an action clears the previous outcome', () => {
    let state = step(IDLE_PANEL, { type: 'start', action: 'quick' });
    state = step(state, { type: 'fail', text: 'Wrong password.', now: 0 });
    state = step(state, { type: 'start', action: 'quick' });
    assert.equal(state.outcome, null);
  });

  test('too many attempts starts a wait for every password on the panel', () => {
    let state = step(IDLE_PANEL, { type: 'start', action: 'changePassword' });
    state = step(state, { type: 'fail', text: '', waitSeconds: 5, now: 10_000 });
    assert.equal(state.waitUntil, 15_000);
    assert.equal(outcomeAt(state, 'changePassword')?.kind, 'wait');
  });

  test('when the wait is over, its message goes but other messages stay', () => {
    let state = step(IDLE_PANEL, { type: 'start', action: 'changePassword' });
    state = step(state, { type: 'fail', text: '', waitSeconds: 5, now: 0 });
    state = step(state, { type: 'waitOver' });
    assert.equal(state.waitUntil, null);
    assert.equal(state.outcome, null);

    state = step(state, { type: 'start', action: 'keyBackup' });
    state = step(state, { type: 'fail', text: 'Wrong password.', now: 0 });
    state = step(state, { type: 'waitOver' });
    assert.equal(outcomeAt(state, 'keyBackup')?.kind, 'error');
  });

  test('an ordinary error does not end a wait that is still running', () => {
    let state = step(IDLE_PANEL, { type: 'start', action: 'changePassword' });
    state = step(state, { type: 'fail', text: '', waitSeconds: 30, now: 0 });
    state = step(state, { type: 'start', action: 'encryptFields' });
    state = step(state, { type: 'fail', text: 'Could not.', now: 1_000 });
    assert.equal(state.waitUntil, 30_000);
  });

  test('success and failure with nothing running change nothing', () => {
    assert.equal(step(IDLE_PANEL, { type: 'succeed', notice: 'x' }), IDLE_PANEL);
    assert.equal(step(IDLE_PANEL, { type: 'fail', text: 'x', now: 0 }), IDLE_PANEL);
  });
});

describe('secondsLeft', () => {
  test('no wait is zero', () => {
    assert.equal(secondsLeft(null, 123), 0);
  });

  test('counts down in whole seconds, rounding up', () => {
    assert.equal(secondsLeft(5_000, 0), 5);
    assert.equal(secondsLeft(5_000, 100), 5);
    assert.equal(secondsLeft(5_000, 4_001), 1);
  });

  test('never goes below zero', () => {
    assert.equal(secondsLeft(5_000, 5_000), 0);
    assert.equal(secondsLeft(5_000, 9_000), 0);
  });
});

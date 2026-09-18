import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { shouldSetUpAutoSummary } from '../../src/lib/meeting-summary-policy.ts';

describe('shouldSetUpAutoSummary', () => {
  test('allows recording summaries when Auto Summary is enabled', () => {
    assert.equal(shouldSetUpAutoSummary('recording', true), true);
  });

  test('blocks recording summaries when Auto Summary is disabled', () => {
    assert.equal(shouldSetUpAutoSummary('recording', false), false);
  });

  test('does not auto-generate when opening an existing meeting', () => {
    assert.equal(shouldSetUpAutoSummary(null, true), false);
  });
});

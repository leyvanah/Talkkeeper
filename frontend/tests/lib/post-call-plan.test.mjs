import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { planPostCall } from '../../src/lib/post-call-plan.ts';

describe('planPostCall', () => {
  test('refines a recording whose audio was saved', () => {
    assert.equal(planPostCall({ hasAudio: true, lineCount: 12 }), 'refine');
  });

  test('refines saved audio even without live text', () => {
    assert.equal(planPostCall({ hasAudio: true, lineCount: 0 }), 'refine');
  });

  test('keeps the live transcript when no audio was saved', () => {
    assert.equal(planPostCall({ hasAudio: false, lineCount: 12 }), 'keep-live');
  });

  test('says nothing was kept when there is neither audio nor text', () => {
    assert.equal(planPostCall({ hasAudio: false, lineCount: 0 }), 'nothing-kept');
  });

  test('runs the pass as before when the audio could not be looked up', () => {
    assert.equal(planPostCall({ hasAudio: null, lineCount: 0 }), 'refine');
    assert.equal(planPostCall({ hasAudio: null, lineCount: null }), 'refine');
  });

  test('does not claim an empty meeting when the lines could not be counted', () => {
    assert.equal(planPostCall({ hasAudio: false, lineCount: null }), 'keep-live');
  });
});

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { mayAskNow } from '../../src/lib/post-call-question.ts';

describe('mayAskNow', () => {
  test('asks when nothing is recording', () => {
    assert.equal(mayAskNow({ isRecording: false, status: 'idle' }), true);
  });

  test('asks once a stopped recording is saved', () => {
    assert.equal(mayAskNow({ isRecording: false, status: 'completed' }), true);
  });

  test('asks while a stopped recording is being saved', () => {
    for (const status of ['stopping', 'processing', 'saving']) {
      assert.equal(mayAskNow({ isRecording: false, status }), true, status);
    }
  });

  test('asks after a recording failed to start', () => {
    assert.equal(mayAskNow({ isRecording: false, status: 'error' }), true);
  });

  test('waits while a recording is running', () => {
    assert.equal(mayAskNow({ isRecording: true, status: 'recording' }), false);
  });

  test('waits while a recording is starting, before the backend reports it', () => {
    assert.equal(mayAskNow({ isRecording: false, status: 'starting' }), false);
  });

  test('waits when the backend reports a recording the status has not caught up with', () => {
    assert.equal(mayAskNow({ isRecording: true, status: 'idle' }), false);
  });
});

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { showsRecordingName } from '../../src/lib/home-title.ts';

describe('home screen title', () => {
  test('names the recording while it is recorded, stopped or saved', () => {
    assert.equal(showsRecordingName(true, 'recording'), true);
    for (const status of ['stopping', 'processing', 'saving', 'completed']) {
      assert.equal(showsRecordingName(false, status), true, status);
    }
  });

  test('after the save, and before the next one is named, it is a new recording', () => {
    assert.equal(showsRecordingName(false, 'idle'), false);
    assert.equal(showsRecordingName(false, 'error'), false);
    // The previous name is still in the context while the next one starts.
    assert.equal(showsRecordingName(false, 'starting'), false);
  });
});

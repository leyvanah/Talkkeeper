import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import {
  lockedRecordingSwitch,
  recommendedLiveModel,
  recommendedPostCallModel,
  shownEchoMode,
} from '../../src/lib/recording-options.ts';

describe('lockedRecordingSwitch', () => {
  test('both on: either may be turned off', () => {
    assert.equal(lockedRecordingSwitch(true, true), null);
  });

  test('the one left on cannot be turned off', () => {
    assert.equal(lockedRecordingSwitch(true, false), 'autoSave');
    assert.equal(lockedRecordingSwitch(false, true), 'liveText');
  });

  test('both off (an old store): nothing is locked, either may be turned on', () => {
    assert.equal(lockedRecordingSwitch(false, false), null);
  });
});

describe('shownEchoMode', () => {
  test('shows the stored choice on Windows', () => {
    assert.equal(shownEchoMode('windows', true), 'windows');
    assert.equal(shownEchoMode('builtin', true), 'builtin');
    assert.equal(shownEchoMode('off', true), 'off');
  });

  test('a store without the choice reads as the default', () => {
    assert.equal(shownEchoMode(undefined, true), 'windows');
  });

  test('elsewhere, Windows cleaning is what the built-in canceller does', () => {
    assert.equal(shownEchoMode('windows', false), 'builtin');
    assert.equal(shownEchoMode(undefined, false), 'builtin');
    assert.equal(shownEchoMode('off', false), 'off');
  });
});

describe('recommended models', () => {
  test('a Russian interface recommends GigaAM in both lists', () => {
    assert.equal(recommendedLiveModel('ru'), 'gigaam');
    assert.equal(recommendedPostCallModel('ru'), 'gigaam');
  });

  test('an English interface keeps Parakeet live and Whisper after', () => {
    assert.equal(recommendedLiveModel('en'), 'parakeet');
    assert.equal(recommendedPostCallModel('en'), 'whisper');
  });
});

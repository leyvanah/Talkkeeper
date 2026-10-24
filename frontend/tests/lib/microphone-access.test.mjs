import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { microphoneProblem, permissionHelpFor, permissionKey } from '../../src/lib/microphone-access.ts';

describe('microphoneProblem', () => {
  test('a listed microphone with access allowed is fine', () => {
    assert.equal(microphoneProblem(1, false), null);
    assert.equal(microphoneProblem(3, false), null);
  });

  test('no input device at all is missing', () => {
    assert.equal(microphoneProblem(0, false), 'missing');
  });

  test('a privacy block wins even though the microphones are still listed', () => {
    assert.equal(microphoneProblem(2, true), 'blocked');
    assert.equal(microphoneProblem(0, true), 'blocked');
  });
});

describe('permissionHelpFor', () => {
  test('talks about the system the app runs on', () => {
    assert.equal(permissionHelpFor('windows'), 'windows');
    assert.equal(permissionHelpFor('macos'), 'macos');
  });

  test('stays neutral elsewhere', () => {
    assert.equal(permissionHelpFor('linux'), 'generic');
    assert.equal(permissionHelpFor('unknown'), 'generic');
  });
});

describe('permissionKey', () => {
  test('picks the variant of this system', () => {
    assert.equal(permissionKey('permissionRequiredMessage', 'windows', ['windows', 'macos']), 'permissionRequiredMessageWindows');
    assert.equal(permissionKey('permissionRequiredMessage', 'macos', ['windows', 'macos']), 'permissionRequiredMessageMacos');
  });

  test('falls back to the neutral text when there is no variant', () => {
    assert.equal(permissionKey('systemAudioNotAvailableMessage', 'windows', ['macos']), 'systemAudioNotAvailableMessage');
    assert.equal(permissionKey('permissionRequiredMessage', 'generic', ['windows', 'macos']), 'permissionRequiredMessage');
  });

  test('never hands macOS wording to Windows', () => {
    assert.equal(permissionKey('micPermissionGrantedCheck', 'windows', ['macos']), 'micPermissionGrantedCheck');
  });
});

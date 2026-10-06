import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { helperExitCode } from '../../src/lib/summary-errors.ts';

describe('helperExitCode', () => {
  test('reads the code of a helper that crashed on Windows', () => {
    assert.equal(helperExitCode('llama-helper exited: exit code: 0xc0000005'), '0xc0000005');
  });

  test('finds the helper error behind what other layers add in front', () => {
    assert.equal(
      helperExitCode('Chunk 1 failed: llama-helper exited: exit status: 3'),
      '3',
    );
  });

  test('knows a helper ended even when the message has no code', () => {
    assert.equal(helperExitCode('llama-helper exited: signal'), '');
  });

  test('leaves other errors alone', () => {
    assert.equal(helperExitCode('Connection refused'), null);
    assert.equal(helperExitCode('Sidecar closed stdout (process may have crashed)'), null);
    assert.equal(helperExitCode(''), null);
    assert.equal(helperExitCode(null), null);
  });
});

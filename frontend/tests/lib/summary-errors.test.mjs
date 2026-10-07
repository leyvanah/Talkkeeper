import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { helperExitCode, summaryErrorText } from '../../src/lib/summary-errors.ts';

const t = (key, values) => (values ? `${key} ${JSON.stringify(values)}` : key);

describe('summaryErrorText', () => {
  test('explains a crashed helper with its code', () => {
    assert.equal(
      summaryErrorText('Summary failed: llama-helper exited: exit code: 0xc0000005', t),
      'genHelperExited {"code":"0xc0000005"}',
    );
  });

  test('marks a missing code instead of leaving a blank', () => {
    assert.equal(summaryErrorText('llama-helper exited: signal', t), 'genHelperExited {"code":"?"}');
  });

  test('explains a provider that is not running', () => {
    assert.equal(summaryErrorText('error sending request: Connection refused', t), 'genConnectionRefused');
  });

  test('passes anything else through', () => {
    assert.equal(summaryErrorText('API key not found for openai', t), 'API key not found for openai');
  });
});

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

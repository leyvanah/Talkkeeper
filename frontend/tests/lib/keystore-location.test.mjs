import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { KEYSTORE_FILE, keystoreFilePath } from '../../src/lib/keystore-location.ts';

describe('keystoreFilePath', () => {
  test('joins a Windows folder with a backslash', () => {
    assert.equal(
      keystoreFilePath('C:\\Program Files\\Talkkeeper\\data'),
      'C:\\Program Files\\Talkkeeper\\data\\keystore.json',
    );
  });

  test('joins a POSIX folder with a slash', () => {
    assert.equal(keystoreFilePath('/opt/talkkeeper/data'), '/opt/talkkeeper/data/keystore.json');
  });

  test('does not double a trailing separator', () => {
    assert.equal(keystoreFilePath('D:\\data\\'), 'D:\\data\\keystore.json');
    assert.equal(keystoreFilePath('C:\\'), 'C:\\keystore.json');
    assert.equal(keystoreFilePath('/srv/data/'), '/srv/data/keystore.json');
  });

  test('with no folder only the file name is left', () => {
    assert.equal(keystoreFilePath(''), KEYSTORE_FILE);
    assert.equal(keystoreFilePath(null), KEYSTORE_FILE);
    assert.equal(keystoreFilePath(undefined), KEYSTORE_FILE);
  });
});

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { MIN_PASSWORD_LENGTH, checkNewPassword } from '../../src/lib/password-rules.ts';

describe('checkNewPassword', () => {
  test('empty fields: nothing to complain about, nothing to submit', () => {
    assert.deepEqual(checkNewPassword('', ''), { tooShort: false, mismatch: false, ok: false });
  });

  test('a password below the minimum is too short', () => {
    const short = 'x'.repeat(MIN_PASSWORD_LENGTH - 1);
    assert.deepEqual(checkNewPassword(short, short), { tooShort: true, mismatch: false, ok: false });
  });

  test('a password of exactly the minimum, repeated, is accepted', () => {
    const exact = 'x'.repeat(MIN_PASSWORD_LENGTH);
    assert.deepEqual(checkNewPassword(exact, exact), { tooShort: false, mismatch: false, ok: true });
  });

  test('without the repeat it cannot be submitted', () => {
    assert.equal(checkNewPassword('correct horse', '').ok, false);
    assert.equal(checkNewPassword('correct horse', '').mismatch, false);
  });

  test('a repeat that is still the beginning of the password is not a mismatch yet', () => {
    const check = checkNewPassword('correct horse', 'correct');
    assert.equal(check.mismatch, false);
    assert.equal(check.ok, false);
  });

  test('a typo in the repeat is a mismatch', () => {
    const check = checkNewPassword('correct horse', 'corect');
    assert.equal(check.mismatch, true);
    assert.equal(check.ok, false);
  });

  test('a repeat longer than the password is a mismatch', () => {
    assert.equal(checkNewPassword('correct horse', 'correct horses').mismatch, true);
  });

  test('case matters', () => {
    assert.equal(checkNewPassword('Correct horse', 'correct horse').mismatch, true);
  });

  test('length is counted in characters, as the backend counts them', () => {
    // Seven emoji are fourteen UTF-16 units but seven characters.
    const emoji = '🔒'.repeat(MIN_PASSWORD_LENGTH - 1);
    assert.equal(checkNewPassword(emoji, emoji).tooShort, true);
    const cyrillic = 'пароль12';
    assert.equal(checkNewPassword(cyrillic, cyrillic).ok, true);
  });
});

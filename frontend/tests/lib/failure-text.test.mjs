import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { failureText } from '../../src/lib/failure-text.ts';

describe('failureText', () => {
  test('a command rejection is a plain string', () => {
    assert.equal(failureText('Database is locked'), 'Database is locked');
  });

  test('a JavaScript error gives its message', () => {
    assert.equal(failureText(new Error('Network down')), 'Network down');
    assert.equal(failureText(new TypeError('')), 'TypeError');
  });

  test('{ message } and { error } objects give their text', () => {
    assert.equal(failureText({ message: 'Not found' }), 'Not found');
    assert.equal(failureText({ error: 'Busy' }), 'Busy');
  });

  test('any other object gives its field names, never the values', () => {
    assert.equal(failureText({ code: 'locked', waitSeconds: 5 }), '{code, waitSeconds}');
    assert.equal(failureText({ message: { text: 'Секретная фраза' } }), '{message}');
    assert.equal(failureText(['Секретная фраза', 'Встреча в четверг']), '[array of 2]');
    assert.equal(failureText({}), '{}');
  });

  test('keys that are data are counted, not written', () => {
    assert.equal(
      failureText({ code: 'x', 'Встреча в четверг': 1, 'Secret phrase': 2 }),
      '{code, +2}',
    );
    const many = Object.fromEntries(Array.from({ length: 10 }, (_, i) => [`field${i}`, i]));
    assert.equal(
      failureText(many),
      '{field0, field1, field2, field3, field4, field5, field6, field7, +2}',
    );
  });

  test('nothing useful still says something', () => {
    assert.equal(failureText(undefined), 'unknown error');
    assert.equal(failureText(null), 'unknown error');
    assert.equal(failureText('   '), 'unknown error');
    assert.equal(failureText(42), '42');
  });

  test('a cyclic object does not throw', () => {
    const cyclic = {};
    cyclic.self = cyclic;
    assert.equal(failureText(cyclic), '{self}');
  });
});

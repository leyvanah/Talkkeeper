import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { archiveIsOpen } from '../../src/lib/archive-gate.ts';

describe('archiveIsOpen', () => {
  test('a locked archive is closed', () => {
    assert.equal(archiveIsOpen({ loading: false, status: { state: 'locked' } }), false);
  });

  test('closed while the state is still being read', () => {
    assert.equal(archiveIsOpen({ loading: true, status: null }), false);
    assert.equal(archiveIsOpen({ loading: true, status: { state: 'unlocked' } }), false);
  });

  test('open when unlocked or never protected', () => {
    assert.equal(archiveIsOpen({ loading: false, status: { state: 'unlocked' } }), true);
    assert.equal(archiveIsOpen({ loading: false, status: { state: 'unconfigured' } }), true);
  });

  test('a status that could not be read leaves the app showing, as the gate does', () => {
    assert.equal(archiveIsOpen({ loading: false, status: null }), true);
  });
});

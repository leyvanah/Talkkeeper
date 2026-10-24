import assert from 'node:assert/strict';
import fs from 'node:fs';
import { describe, test } from 'node:test';
import { LIBRARY_TRANSFER_AVAILABLE, libraryActions } from '../../src/lib/library-transfer.ts';

const card = fs.readFileSync(
  new URL('../../src/components/security/LibraryExportCard.tsx', import.meta.url),
  'utf8',
);

describe('library transfer', () => {
  test('is off for the first release', () => {
    assert.equal(LIBRARY_TRANSFER_AVAILABLE, false);
  });

  test('while off, the card offers only the readable export', () => {
    assert.deepEqual(libraryActions(), ['readable']);
    assert.deepEqual(libraryActions(false), ['readable']);
  });

  test('turned on, package and import come back after the readable export', () => {
    assert.deepEqual(libraryActions(true), ['readable', 'package', 'import']);
  });

  test('the card shows the package and import buttons only when offered', () => {
    assert.match(card, /libraryActions\(\)/);
    assert.match(card, /actions\.includes\('package'\) && \(/);
    assert.match(card, /actions\.includes\('import'\) && \(/);
  });
});

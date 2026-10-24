import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { formatDiskSize, noSpaceText, parseNoSpace } from '../../src/lib/disk-space.ts';

const GIB = 1024 ** 3;
const MIB = 1024 ** 2;

describe('parseNoSpace', () => {
  test('reads both numbers, wherever a command put the marker', () => {
    const raw = `NO_DISK_SPACE needed=${3 * GIB} available=${GIB}: not enough free disk space`;
    assert.deepEqual(parseNoSpace(raw), { needed: 3 * GIB, available: GIB });
    assert.deepEqual(parseNoSpace(`Failed to download model: ${raw}`), {
      needed: 3 * GIB,
      available: GIB,
    });
    assert.deepEqual(parseNoSpace(new Error(raw)), { needed: 3 * GIB, available: GIB });
  });

  test('any other error is not a lack of space', () => {
    assert.equal(parseNoSpace('Download timeout - No data received for 30 seconds'), null);
    assert.equal(parseNoSpace('Storage error'), null);
    assert.equal(parseNoSpace(undefined), null);
  });
});

describe('formatDiskSize', () => {
  test('gigabytes and megabytes in the interface language', () => {
    assert.equal(formatDiskSize(2.6 * GIB, 'ru'), '2,6 ГБ');
    assert.equal(formatDiskSize(2.6 * GIB, 'en'), '2.6 GB');
    assert.equal(formatDiskSize(750 * MIB, 'en'), '750 MB');
  });
});

describe('noSpaceText', () => {
  test('fills the phrase with formatted sizes', () => {
    const text = noSpaceText(
      `NO_DISK_SPACE needed=${2 * GIB} available=${512 * MIB}`,
      'en',
      ({ needed, available }) => `${needed} needed, ${available} free`,
    );
    assert.equal(text, '2 GB needed, 512 MB free');
  });

  test('leaves other errors to the caller', () => {
    assert.equal(noSpaceText('Connection failed', 'ru', () => 'x'), null);
  });
});

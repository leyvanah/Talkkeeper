import assert from 'node:assert/strict';
import { afterEach, describe, mock, test } from 'node:test';
import { deleteEach } from '../../src/lib/bulk-delete.ts';

const originalConsoleError = console.error;

/** Deletes everything except the ids in `refused`. */
const backend = (refused = []) =>
  mock.fn(async (id) => {
    if (refused.includes(id)) throw new Error('database is locked');
  });

describe('deleteEach', () => {
  afterEach(() => {
    console.error = originalConsoleError;
    mock.restoreAll();
  });

  test('reports every meeting deleted when all go', async () => {
    const deleteOne = backend();

    const result = await deleteEach(['a', 'b', 'c'], deleteOne);

    assert.deepEqual(result, { deleted: ['a', 'b', 'c'], failed: [] });
    assert.equal(deleteOne.mock.callCount(), 3);
  });

  test('keeps going after a failure and names what did not go', async () => {
    console.error = mock.fn(() => {});
    const deleteOne = backend(['b', 'd']);

    const result = await deleteEach(['a', 'b', 'c', 'd', 'e'], deleteOne);

    assert.deepEqual(result, { deleted: ['a', 'c', 'e'], failed: ['b', 'd'] });
    assert.equal(deleteOne.mock.callCount(), 5);
  });

  test('reports all failed when nothing goes', async () => {
    console.error = mock.fn(() => {});

    const result = await deleteEach(['a', 'b'], backend(['a', 'b']));

    assert.deepEqual(result, { deleted: [], failed: ['a', 'b'] });
  });
});

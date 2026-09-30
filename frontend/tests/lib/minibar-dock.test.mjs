import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { dockedTabPaths, REVEAL_RANGE, revealFor } from '../../src/lib/minibar-dock.ts';

describe('revealFor', () => {
  test('is fully out with the cursor on the bar or while held', () => {
    assert.equal(revealFor(0, false), 1);
    assert.equal(revealFor(500, true), 1);
  });

  test('is tucked away beyond the range', () => {
    assert.equal(revealFor(REVEAL_RANGE, false), 0);
    assert.equal(revealFor(REVEAL_RANGE * 3, false), 0);
  });

  test('comes out more as the cursor comes closer, gently at first', () => {
    const far = revealFor(REVEAL_RANGE * 0.75, false);
    const half = revealFor(REVEAL_RANGE * 0.5, false);
    const near = revealFor(REVEAL_RANGE * 0.1, false);
    assert.ok(far < half && half < near && near < 1);
    assert.ok(half < 0.5);
  });
});

describe('dockedTabPaths', () => {
  const { fill, outline } = dockedTabPaths(440, 48, 10, 16);

  test('the fill runs along the whole top edge and closes', () => {
    assert.match(fill, /^M0 0 H440 /);
    assert.match(fill, / Z$/);
  });

  test('the outline leaves the top edge to the screen', () => {
    assert.match(outline, /^M0 0 A/);
    assert.match(outline, /A10\.5 10\.5 0 0 1 440 0$/);
    assert.doesNotMatch(outline, /H440/);
  });

  test('the top corners curve outwards, the bottom ones inwards', () => {
    // Fill: ears are drawn against the direction of travel, bottom corners with it.
    const arcs = [...fill.matchAll(/A(\S+) \S+ 0 0 (\d)/g)].map(([, radius, sweep]) => [Number(radius), sweep]);
    assert.deepEqual(arcs, [[10.5, '0'], [16, '1'], [16, '1'], [10.5, '0']]);
  });
});

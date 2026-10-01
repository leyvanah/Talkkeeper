import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { barPaths, PEEL_DISTANCE, peelFor, REVEAL_RANGE, revealFor } from '../../src/lib/minibar-dock.ts';

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

describe('peelFor', () => {
  test('a free bar is a pill, a docked one a tab until it comes away', () => {
    assert.equal(peelFor(false, 0), 1);
    assert.equal(peelFor(true, 0), 0);
    assert.equal(peelFor(true, PEEL_DISTANCE / 2), 0.5);
    assert.equal(peelFor(true, PEEL_DISTANCE * 4), 1);
    assert.equal(peelFor(true, -3), 0);
  });
});

describe('barPaths', () => {
  const W = 440;
  const H = 48;
  const numbers = (path) => path.match(/-?\d+(\.\d+)?/g).map(Number);
  const commands = (path) => path.replace(/[^A-Za-z]/g, '');

  test('docked, the top edge runs the whole width along the screen edge', () => {
    const { fill, top } = barPaths(W, H, 10, 16, 0);
    assert.match(fill, /^M0 0 L440 0 /);
    assert.equal(top, 'M0 0 L440 0');
  });

  test('free, it is a pill standing in from the sides, rounded all round', () => {
    const { fill, top } = barPaths(W, H, 10, 16, 1);
    const pill = (H - 1) / 2;
    assert.ok(fill.startsWith(`M${10.5 + pill} 0.5 L${W - 10.5 - pill} 0.5 `), fill);
    assert.equal(top, `M${10.5 + pill} 0.5 L${W - 10.5 - pill} 0.5`);
  });

  test('every stage is drawn with the same commands, so it can be eased between', () => {
    const shapes = [0, 0.3, 0.7, 1].map((peel) => barPaths(W, H, 10, 16, peel));
    for (const key of ['fill', 'outline', 'top']) {
      const first = commands(shapes[0][key]);
      for (const shape of shapes) assert.equal(commands(shape[key]), first);
    }
  });

  test('stays inside the box and is symmetric', () => {
    for (const peel of [0, 0.5, 1]) {
      const values = numbers(barPaths(W, H, 10, 16, peel).fill);
      const xs = values.filter((_, i) => i % 2 === 0);
      const ys = values.filter((_, i) => i % 2 === 1);
      assert.ok(Math.min(...xs) >= 0 && Math.max(...xs) <= W);
      assert.ok(Math.min(...ys) >= 0 && Math.max(...ys) <= H);
      assert.ok(Math.abs(Math.min(...xs) - (W - Math.max(...xs))) < 0.01);
    }
  });
});

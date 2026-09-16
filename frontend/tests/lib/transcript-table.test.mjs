// Run with: node tests/lib/transcript-table.test.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import ts from 'typescript';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const modulePath = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  '..',
  '..',
  'src',
  'lib',
  'transcript-table.ts'
);
const require = createRequire(import.meta.url);

function loadTsModule(filePath) {
  const source = fs.readFileSync(filePath, 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2020,
    },
  }).outputText;

  const module = { exports: {} };
  vm.runInNewContext(compiled, {
    exports: module.exports,
    module,
    require,
  });
  return module.exports;
}

const { layoutTimeline, yAt, timeAt, ticksBetween } = loadTsModule(modulePath);

const sideOf = (speaker) => {
  if (speaker === 'You') return 'host';
  if (speaker === 'You + Speaker 1') return 'both';
  return 'client';
};
const line = (id, start, end, speaker, height = 40) => ({ id, start, end, speaker, text: id, height });
const heightOf = (l) => l.height;
const options = { pxPerSecond: 20, gap: 8, padding: 10 };
const layout = (lines) => layoutTimeline(lines, sideOf, heightOf, options);
const topOf = (result, id) => result.placed.find((p) => p.line.id === id).top;
const near = (actual, expected, message) =>
  assert.ok(Math.abs(actual - expected) < 1e-6, `${message}: ${actual} != ${expected}`);

// Where nothing is crowded, time is proportional.
{
  const result = layout([line('a', 0, 1, 'Speaker 1'), line('b', 10, 11, 'You')]);
  assert.equal(topOf(result, 'a'), 10);
  assert.equal(topOf(result, 'b'), 10 + 10 * 20);
  near(yAt(result, 5), 10 + 5 * 20, 'a second in the middle');
}

// Every line sits on the ruler at its own start, even when text pushes things
// down: the ruler stretches instead.
{
  const result = layout([
    line('a', 0, 3, 'Speaker 1', 200),
    line('b', 1, 2, 'You'),
    line('c', 2, 3, 'Speaker 1'),
  ]);
  for (const id of ['a', 'b', 'c']) {
    const placed = result.placed.find((p) => p.line.id === id);
    near(placed.top, yAt(result, placed.line.start), `${id} is on its own tick`);
  }
  // b is an interruption: it is beside a, not below it.
  assert.ok(topOf(result, 'b') < topOf(result, 'a') + 200);
  // c follows a in the same column, so the stretch 1s..2s was lengthened.
  assert.ok(topOf(result, 'c') >= topOf(result, 'a') + 200 + 8);
}

// Nothing in one column overlaps.
{
  const lines = [];
  for (let i = 0; i < 50; i++) {
    lines.push(line(`h${i}`, i * 0.7, i * 0.7 + 0.5, 'You', 30 + (i % 4) * 25));
    lines.push(line(`c${i}`, i * 0.9, i * 0.9 + 0.3, 'Speaker 1', 30 + (i % 3) * 40));
  }
  const result = layout(lines);
  for (const column of ['host', 'client']) {
    const own = result.placed
      .filter((p) => p.column === column || p.column === 'both')
      .sort((a, b) => a.top - b.top);
    for (let i = 1; i < own.length; i++) {
      assert.ok(own[i].top >= own[i - 1].top + own[i - 1].height, `${column} lines overlap`);
    }
  }
  for (const placed of result.placed) {
    near(placed.top, yAt(result, placed.line.start), `${placed.line.id} is on its own tick`);
  }
  // The ruler only ever goes down.
  for (let i = 1; i < result.anchors.length; i++) {
    assert.ok(result.anchors[i].y > result.anchors[i - 1].y);
    assert.ok(result.anchors[i].t > result.anchors[i - 1].t);
  }
}

// Two sides starting together share a position; if one side is crowded, both move.
{
  const result = layout([
    line('a', 0, 0.5, 'Speaker 1', 200),
    line('b', 1, 2, 'Speaker 1'),
    line('c', 1, 2, 'You'),
  ]);
  assert.equal(topOf(result, 'b'), topOf(result, 'c'));
  assert.ok(topOf(result, 'b') >= 10 + 200 + 8);
}

// Two lines of one side at the same instant stack.
{
  const result = layout([line('a', 3, 4, 'You'), line('b', 3, 4, 'You')]);
  assert.equal(topOf(result, 'b'), topOf(result, 'a') + 40 + 8);
}

// Both at once takes both columns, so neither side can sit on it.
{
  const result = layout([line('x', 0, 1, 'You + Speaker 1', 100), line('y', 1, 2, 'Speaker 1')]);
  assert.equal(result.placed[0].column, 'both');
  assert.ok(topOf(result, 'y') >= topOf(result, 'x') + 100);
}

// Position and time convert both ways.
{
  const result = layout([line('a', 0, 3, 'Speaker 1', 200), line('b', 5, 9, 'You')]);
  for (const t of [0, 0.5, 2, 4.9, 5, 7, 12]) {
    near(timeAt(result, yAt(result, t)), t, `round trip at ${t}s`);
  }
  assert.ok(result.height >= yAt(result, 9));
}

// A ruler mark every second, labels spaced by scale, tenths when zoomed in.
{
  const result = layout([line('a', 0, 30, 'Speaker 1')]);
  const ticks = ticksBetween(result, 0, yAt(result, 10));
  assert.deepEqual(Array.from(ticks, (tick) => tick.t), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
  assert.deepEqual(
    Array.from(ticks.filter((tick) => tick.kind === 'label'), (tick) => tick.t),
    [0, 10],
  );

  const zoomed = layoutTimeline([line('a', 0, 30, 'Speaker 1')], sideOf, heightOf, {
    ...options,
    pxPerSecond: 80,
  });
  const fine = ticksBetween(zoomed, yAt(zoomed, 1), yAt(zoomed, 2));
  assert.deepEqual(
    Array.from(fine, (tick) => tick.t),
    [1, 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 1.9, 2],
  );
  assert.equal(fine[0].kind, 'label');
  assert.equal(fine[1].kind, 'minor');
}

// Nothing to lay out still gives a usable ruler.
{
  const result = layout([]);
  assert.equal(result.placed.length, 0);
  assert.ok(result.height > 0);
}

console.log('transcript-table: ok');

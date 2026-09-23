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

// Text is cut into sentences, and long sentences further.
{
  const { splitForTimeline } = loadTsModule(modulePath);
  assert.deepEqual(
    Array.from(splitForTimeline('Первое. Второе?  Третье!')),
    ['Первое.', 'Второе?', 'Третье!'],
  );
  const long = 'слово '.repeat(40).trim();
  const cut = Array.from(splitForTimeline(long, 30));
  assert.ok(cut.length > 1);
  assert.ok(cut.every((piece) => piece.length <= 30), 'no piece is longer than asked');
  assert.equal(cut.join(' '), long, 'nothing is lost or reordered');
  assert.deepEqual(
    Array.from(splitForTimeline('раз, два, три, четыре, пять, шесть', 20)),
    ['раз, два, три,', 'четыре, пять, шесть'],
    'a comma is preferred',
  );
  assert.deepEqual(Array.from(splitForTimeline('ооооооооооооооооооооооо', 5)), ['ооооооооооооооооооооооо']);
}

// Pieces sit at the share of the line their text takes, unless the piece above
// is in the way.
{
  const { placePieces } = loadTsModule(modulePath);
  const straight = (fraction) => fraction * 300;
  near(
    placePieces(['aaaa', 'bbbb', 'cccc'], [20, 20, 20], straight)[1],
    100,
    'a third of the text is a third of the way down',
  );
  const crowded = Array.from(placePieces(['aaaa', 'bbbb', 'cccc'], [150, 150, 150], straight, 4));
  assert.deepEqual(crowded, [0, 154, 308], 'too much text stacks instead of overlapping');
}

console.log('transcript-table pieces: ok');

// Near the end of the box, pieces move up only as far as they must to fit.
{
  const { placePieces } = loadTsModule(modulePath);
  const straight = (fraction) => fraction * 300;
  const fitted = Array.from(placePieces(['aaaa', 'bbbb', 'cccc'], [20, 20, 90], straight, 4, 260));
  assert.deepEqual(fitted.slice(0, 2), [0, 100], 'pieces with room stay on their time');
  assert.equal(fitted[2], 170, 'the last one is lifted just enough to end at the bottom');
  const tight = Array.from(placePieces(['aaaa', 'bbbb'], [100, 100], straight, 4, 204));
  assert.deepEqual(tight, [0, 104], 'exactly enough room packs them');
}

console.log('transcript-table fit: ok');

// Timed words become pieces at sentence ends, each starting when its first word did.
{
  const { piecesFromWords } = loadTsModule(modulePath);
  const words = [
    { w: 'Первое', s: 0.5, e: 0.9 },
    { w: 'слово.', s: 0.9, e: 1.4 },
    { w: 'Второе', s: 3.2, e: 3.6 },
    { w: 'предложение', s: 3.6, e: 4.4 },
  ];
  const pieces = Array.from(piecesFromWords(words));
  assert.deepEqual(pieces.map((p) => p.text), ['Первое слово.', 'Второе предложение']);
  assert.deepEqual(pieces.map((p) => p.start), [0.5, 3.2]);
  const short = Array.from(piecesFromWords(words, 12));
  assert.ok(short.every((p) => p.text.length <= 12 || p.words.length === 1));
  assert.equal(Array.from(piecesFromWords([])).length, 0);
}

// Placing at given offsets follows the same rules as the estimates.
{
  const { placeAt } = loadTsModule(modulePath);
  assert.deepEqual(Array.from(placeAt([0, 100, 110], [20, 20, 20], 4)), [0, 100, 124]);
  assert.deepEqual(Array.from(placeAt([0, 100, 200], [20, 20, 90], 4, 260)), [0, 100, 170]);
}

// The words sounding at a moment, from both sides.
{
  const { wordsAt } = loadTsModule(modulePath);
  const words = [
    { w: 'a', s: 0, e: 1 },
    { w: 'b', s: 1, e: 3 },
    { w: 'c', s: 2, e: 2.5 },
    { w: 'd', s: 4, e: 5 },
  ];
  assert.deepEqual(Array.from(wordsAt(words, 0.5)), [0]);
  assert.deepEqual(Array.from(wordsAt(words, 2.2)), [1, 2], 'overlapping words both sound');
  assert.deepEqual(Array.from(wordsAt(words, 3.5)), [], 'a pause sounds nothing');
  assert.deepEqual(Array.from(wordsAt(words, 1)), [1], 'a word ends where the next begins');
  assert.deepEqual(Array.from(wordsAt([], 1)), []);
}

console.log('transcript-table words: ok');

// Long pauses are shortened; short ones and speech keep their time.
{
  const { buildWarp, toShown, toReal } = loadTsModule(modulePath);
  const quiet = { keep: 1, rate: 0.1, most: 3 };
  const lines = [
    line('a', 20, 22, 'Speaker 1'),
    line('b', 22.5, 24, 'You'),
    line('c', 30, 31, 'Speaker 1'),
    line('d', 200, 201, 'You'),
  ];
  const warp = buildWarp(lines, quiet);
  // 20 s of silence at the start: 1 + 19 * 0.1 = 2.9 s.
  near(toShown(warp, 20), 2.9, 'the opening pause');
  near(toShown(warp, 22.5) - toShown(warp, 22), 0.5, 'a short pause stays');
  near(toShown(warp, 30) - toShown(warp, 24), 1.5, 'a 6 s pause becomes 1.5 s');
  near(toShown(warp, 200) - toShown(warp, 31), 3, 'no pause is longer than most');
  near(toShown(warp, 200.5) - toShown(warp, 200), 0.5, 'speech runs at its own pace');
  for (const t of [0, 7, 20, 23, 27, 100, 200.7]) near(toReal(warp, toShown(warp, t)), t, `round trip ${t}`);

  // Word times are what counts as speech, not the span of the line.
  const worded = { ...line('w', 0, 30, 'Speaker 1'), words: [{ w: 'x', s: 0, e: 1 }, { w: 'y', s: 25, e: 26 }] };
  const byWords = buildWarp([worded], quiet);
  near(toShown(byWords, 25), 1 + 3, 'a pause inside a line is shortened');

  const result = layoutTimeline(lines, sideOf, heightOf, { ...options, quiet });
  assert.equal(result.quiet.length, 3);
  near(topOf(result, 'a'), 10 + 2.9 * 20, 'the first line after the opening pause');
  for (const placed of result.placed) {
    near(placed.top, yAt(result, placed.line.start), `${placed.line.id} is on its own tick`);
  }
  for (const y of [10, 50, 200, 400]) near(yAt(result, timeAt(result, y)), y, `screen round trip ${y}`);
  const ticks = ticksBetween(result, 0, result.height);
  assert.ok(!ticks.some((tick) => tick.t > 31 && tick.t < 200), 'no marks inside a shortened pause');
  for (let i = 1; i < ticks.length; i++) assert.ok(ticks[i].y - ticks[i - 1].y >= 3, 'marks do not crowd');

  // Without the option nothing changes.
  const plain = layout(lines);
  near(topOf(plain, 'd'), yAt(plain, 200), 'plain layout');
  assert.equal(plain.quiet.length, 0);
}

console.log('transcript-table quiet: ok');

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

const { buildTableRows, rowIndexOf } = loadTsModule(modulePath);

const sideOf = (speaker) => {
  if (speaker === 'You') return 'host';
  if (speaker === 'You + Speaker 1') return 'both';
  return 'client';
};
const line = (id, start, end, speaker) => ({ id, start, end, speaker, text: id });
// Arrays made inside the vm context have its prototypes; copy them out so
// deepStrictEqual compares values.
const ids = (lines) => Array.from(lines, (l) => l.id);

// Turns that follow one another are rows of their own.
{
  const rows = buildTableRows(
    [line('a', 0, 2, 'Speaker 1'), line('b', 2, 4, 'You'), line('c', 5, 6, 'Speaker 1')],
    sideOf,
  );
  assert.equal(rows.length, 3, 'a turn that starts when the last ended is not an interruption');
  assert.deepEqual(ids(rows[0].client), ['a']);
  assert.deepEqual(ids(rows[0].host), []);
  assert.deepEqual(ids(rows[1].host), ['b']);
}

// An interruption shares the row of the line it cut into.
{
  const rows = buildTableRows(
    [line('a', 0, 5, 'Speaker 1'), line('b', 3, 4, 'You'), line('c', 6, 7, 'You')],
    sideOf,
  );
  assert.equal(rows.length, 2);
  assert.deepEqual(ids(rows[0].client), ['a']);
  assert.deepEqual(ids(rows[0].host), ['b']);
  assert.equal(rows[0].start, 0);
  assert.equal(rows[0].end, 5);
}

// Overlap chains: c overlaps b, which overlaps a, so all three are one row even
// though c starts after a ended.
{
  const rows = buildTableRows(
    [line('a', 0, 3, 'Speaker 1'), line('b', 2, 6, 'You'), line('c', 5, 8, 'Speaker 1')],
    sideOf,
  );
  assert.equal(rows.length, 1);
  assert.deepEqual(ids(rows[0].client), ['a', 'c']);
  assert.equal(rows[0].end, 8);
}

// The two tracks arrive out of order; the table does not care.
{
  const rows = buildTableRows(
    [line('b', 3, 4, 'You'), line('a', 0, 5, 'Speaker 1')],
    sideOf,
  );
  assert.equal(rows.length, 1);
  assert.equal(rows[0].key, 'a', 'a row is named after its first line in time');
}

// Both at once fills both columns.
{
  const rows = buildTableRows([line('x', 0, 1, 'You + Speaker 1')], sideOf);
  assert.deepEqual(ids(rows[0].host), ['x']);
  assert.deepEqual(ids(rows[0].client), ['x']);
  assert.deepEqual(Array.from(rows[0].ids), ['x']);
}

// A line with no end is a point, and does not swallow what follows.
{
  const rows = buildTableRows([line('a', 1, undefined, 'You'), line('b', 1.5, 2, 'Speaker 1')], sideOf);
  assert.equal(rows.length, 2);
}

// Finding the row the recording is at.
{
  const rows = buildTableRows(
    [line('a', 0, 5, 'Speaker 1'), line('b', 3, 4, 'You'), line('c', 6, 7, 'You')],
    sideOf,
  );
  assert.equal(rowIndexOf(rows, 'b'), 0);
  assert.equal(rowIndexOf(rows, 'c'), 1);
  assert.equal(rowIndexOf(rows, 'nope'), -1);
  assert.equal(rowIndexOf(rows, null), -1);
}

assert.equal(buildTableRows([], sideOf).length, 0);

console.log('transcript-table: ok');

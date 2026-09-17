// Run with: node tests/lib/playhead.test.mjs
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
  'playhead.ts'
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


const { createPlayhead } = loadTsModule(modulePath);

// A subscriber hears where things stand at once, then each change.
{
  const playhead = createPlayhead();
  playhead.update(3, true);
  const heard = [];
  const stop = playhead.subscribe((t, playing) => heard.push(`${t}:${playing}`));
  playhead.update(3.5, true);
  playhead.update(3.5, true); // no change, no call
  playhead.update(3.5, false);
  stop();
  playhead.update(9, true);
  assert.deepEqual(heard, ['3:true', '3.5:true', '3.5:false']);
  assert.equal(playhead.time(), 9);
  assert.equal(playhead.playing(), true);
}

// Several listeners, each unsubscribing on its own.
{
  const playhead = createPlayhead();
  let a = 0;
  let b = 0;
  const stopA = playhead.subscribe(() => a++);
  playhead.subscribe(() => b++);
  stopA();
  playhead.update(1, false);
  assert.equal(a, 1);
  assert.equal(b, 2);
}

console.log('playhead: ok');

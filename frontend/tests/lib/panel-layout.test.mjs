// Run with: node tests/lib/panel-layout.test.mjs
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
  'panel-layout.ts'
);
const require = createRequire(import.meta.url);

function loadTsModule(filePath, window) {
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
    window,
  });
  return module.exports;
}

const store = new Map();
const window = {
  localStorage: {
    getItem: (k) => (store.has(k) ? store.get(k) : null),
    setItem: (k, v) => store.set(k, String(v)),
  },
};
const L = loadTsModule(modulePath, window);
const plain = (v) => JSON.parse(JSON.stringify(v));

// Nothing stored, or something unreadable, gives the default layout.
{
  assert.deepEqual(plain(L.parseLayout(null)), plain(L.DEFAULT_LAYOUT));
  assert.deepEqual(plain(L.parseLayout('not json')), plain(L.DEFAULT_LAYOUT));
  assert.deepEqual(plain(L.parseLayout('42')), plain(L.DEFAULT_LAYOUT));
}

// Out-of-range and wrong-typed values are replaced one by one.
{
  const layout = L.parseLayout(
    JSON.stringify({ sidebarWidth: 9000, sidebarCollapsed: 'yes', split: 0.1, panes: 'neither' })
  );
  assert.equal(layout.sidebarWidth, L.SIDEBAR_MAX_WIDTH);
  assert.equal(layout.sidebarCollapsed, false);
  assert.equal(layout.split, L.SPLIT_MIN);
  assert.equal(layout.panes, 'both');

  const kept = L.parseLayout(
    JSON.stringify({ sidebarWidth: 300.4, sidebarCollapsed: true, split: 0.6, panes: 'summary' })
  );
  assert.deepEqual(plain(kept), { sidebarWidth: 300, sidebarCollapsed: true, split: 0.6, panes: 'summary' });
}

// A saved layout comes back; failing storage is not an error.
{
  const layout = { sidebarWidth: 320, sidebarCollapsed: true, split: 0.4, panes: 'transcript' };
  L.saveLayout(layout);
  assert.deepEqual(plain(L.loadLayout()), layout);

  const broken = loadTsModule(modulePath, {
    localStorage: {
      getItem: () => {
        throw new Error('denied');
      },
      setItem: () => {
        throw new Error('denied');
      },
    },
  });
  broken.saveLayout(layout);
  assert.deepEqual(plain(broken.loadLayout()), plain(broken.DEFAULT_LAYOUT));
}

// The sidebar takes its width, or the icon strip when collapsed.
{
  assert.equal(L.sidebarOffset({ sidebarWidth: 300, sidebarCollapsed: false }), 300);
  assert.equal(L.sidebarOffset({ sidebarWidth: 300, sidebarCollapsed: true }), L.SIDEBAR_COLLAPSED_WIDTH);
  assert.equal(L.clampSidebarWidth(10), L.SIDEBAR_MIN_WIDTH);
}

// The divider follows the pointer, within bounds.
{
  assert.equal(L.splitAt(600, 100, 1000), 0.5);
  assert.equal(L.splitAt(100, 100, 1000), L.SPLIT_MIN);
  assert.equal(L.splitAt(1100, 100, 1000), L.SPLIT_MAX);
  assert.equal(L.splitAt(500, 100, 0), L.SPLIT_DEFAULT);
}

// Hiding a column leaves the other; hiding again, or hiding the only one, shows both.
{
  assert.equal(L.togglePane('both', 'summary'), 'transcript');
  assert.equal(L.togglePane('both', 'transcript'), 'summary');
  assert.equal(L.togglePane('transcript', 'summary'), 'both');
  assert.equal(L.togglePane('transcript', 'transcript'), 'both');
  assert.equal(L.isPaneShown('both', 'summary'), true);
  assert.equal(L.isPaneShown('transcript', 'summary'), false);
  assert.equal(L.isPaneShown('summary', 'summary'), true);
}

console.log('panel-layout tests passed');

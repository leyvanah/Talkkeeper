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

// The divider follows the pointer, and neither column goes below its
// minimum width — unless the row cannot hold two of them.
{
  assert.equal(L.splitAt(600, 100, 1000), 0.5);
  // A window wide enough for the share limits to be what binds.
  assert.equal(L.splitAt(0, 0, 2000), L.SPLIT_MIN);
  assert.equal(L.splitAt(2000, 0, 2000), L.SPLIT_MAX);
  assert.equal(L.splitAt(100, 100, 1000), L.PANE_MIN_WIDTH / 1000);
  assert.equal(L.splitAt(1100, 100, 1000), 1 - L.PANE_MIN_WIDTH / 1000);
  assert.equal(L.splitAt(500, 100, 0), L.SPLIT_DEFAULT);
  // 800 wide: a quarter of it would leave 200px, under the 280px minimum.
  assert.equal(L.splitAt(150, 0, 800), L.PANE_MIN_WIDTH / 800);
  assert.equal(L.splitAt(700, 0, 800), 1 - L.PANE_MIN_WIDTH / 800);
  assert.equal(L.splitAt(100, 0, 500), 0.5);
}

// Dragging the sidebar's edge in collapses it; dragging it out opens it again.
{
  assert.deepEqual(plain(L.sidebarFromDrag(320)), { collapsed: false, width: 320 });
  assert.deepEqual(plain(L.sidebarFromDrag(40)), { collapsed: true, width: L.SIDEBAR_MIN_WIDTH });
  assert.equal(L.sidebarFromDrag(L.SIDEBAR_COLLAPSE_AT).collapsed, false);
  assert.equal(L.sidebarFromDrag(L.SIDEBAR_COLLAPSE_AT).width, L.SIDEBAR_MIN_WIDTH);
  assert.equal(L.sidebarFromDrag(9000).width, L.SIDEBAR_MAX_WIDTH);
}

// Dragging the divider over a column hides it, leaving the other one whole.
{
  assert.equal(L.panesFromDrag(600, 100, 1000).panes, 'both');
  assert.equal(L.panesFromDrag(150, 100, 1000).panes, 'summary');
  assert.equal(L.panesFromDrag(1050, 100, 1000).panes, 'transcript');
  // Just short of hiding, the column is still at its minimum width.
  const kept = L.panesFromDrag(100 + L.PANE_COLLAPSE_AT, 100, 1000);
  assert.equal(kept.panes, 'both');
  assert.equal(kept.split, L.PANE_MIN_WIDTH / 1000);
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

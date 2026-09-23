import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import ts from 'typescript';
import { beforeEach, describe, mock, test } from 'node:test';
import { fileURLToPath } from 'node:url';

const srcDir = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..', 'src');

/** Loads a TS module from src/ with `@/` aliases resolved and Tauri replaced. */
function loadModule(relative, context) {
  const source = fs.readFileSync(path.join(srcDir, relative), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText;
  const module = { exports: {} };
  const require = (specifier) => {
    if (specifier === '@tauri-apps/api/core') return { invoke: context.invoke };
    if (specifier.startsWith('@/')) return loadModule(`${specifier.slice(2)}.ts`, context);
    throw new Error(`unexpected import ${specifier}`);
  };
  vm.runInNewContext(compiled, { exports: module.exports, module, require, window: context.window });
  return module.exports;
}

function localStorageWindow(values) {
  return {
    localStorage: {
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => values.set(key, value),
      removeItem: (key) => values.delete(key),
      clear: () => values.clear(),
    },
  };
}

const failingWindow = {
  localStorage: {
    getItem: () => null,
    setItem: () => {
      throw new Error('quota exceeded');
    },
    removeItem: () => {},
    clear: () => {},
  },
};

const noFolder = { language: null, storage: 'local_fallback' };

/** The module runs in its own vm realm; copy results into this one to compare. */
const plain = (value) => ({ ...value });

describe('summary language local fallback', () => {
  let values;
  let invoke;
  let prefs;

  beforeEach(() => {
    values = new Map();
    invoke = mock.fn(async () => noFolder);
    prefs = loadModule('lib/summary-language-preferences.ts', {
      invoke,
      window: localStorageWindow(values),
    });
  });

  test('reads summary language from local fallback when meeting has no folder', async () => {
    values.set('summaryLanguageFallback:meeting-1', 'fr');
    assert.deepEqual(plain(await prefs.readMeetingSummaryLanguage('meeting-1')), {
      language: 'fr',
      storage: 'local_fallback',
    });
  });

  test('saves summary language locally when command reports no folder', async () => {
    assert.deepEqual(plain(await prefs.saveMeetingSummaryLanguage('meeting-1', 'es')), {
      language: 'es',
      storage: 'local_fallback',
    });
    assert.equal(values.get('summaryLanguageFallback:meeting-1'), 'es');
  });

  test('clears local fallback when Auto is saved for a folderless meeting', async () => {
    values.set('summaryLanguageFallback:meeting-1', 'de');
    assert.deepEqual(plain(await prefs.saveMeetingSummaryLanguage('meeting-1', null)), {
      language: null,
      storage: 'local_fallback',
    });
    assert.equal(values.has('summaryLanguageFallback:meeting-1'), false);
  });

  test('caches detected language locally when meeting has no folder', async () => {
    await prefs.saveCachedDetectedSummaryLanguage('meeting-1', 'pt');
    assert.equal(values.get('detectedSummaryLanguageFallback:meeting-1'), 'pt');
  });

  test('rejects when folderless summary language cannot be persisted locally', async () => {
    prefs = loadModule('lib/summary-language-preferences.ts', { invoke, window: failingWindow });
    await assert.rejects(
      prefs.saveMeetingSummaryLanguage('meeting-1', 'it'),
      /Failed to save summary language on this device/,
    );
  });
});

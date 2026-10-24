import assert from 'node:assert/strict';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, test } from 'node:test';

// A key missing from the dictionaries does not fail the build: next-intl shows
// the key's path on screen instead. This reads every component for
// `const t = useTranslations('ns')` and then `t('key')` with a literal key, and
// checks each one against both dictionaries.

const root = new URL('../../', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const dictionaries = {
  ru: JSON.parse(readFileSync(join(root, 'messages/ru.json'), 'utf8')),
  en: JSON.parse(readFileSync(join(root, 'messages/en.json'), 'utf8')),
};

function has(dictionary, path) {
  let node = dictionary;
  for (const part of path.split('.')) {
    if (node === null || typeof node !== 'object' || !(part in node)) return false;
    node = node[part];
  }
  return typeof node === 'string';
}

function sources(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.(tsx?|jsx?)$/.test(name) ? [path] : [];
  });
}

function usedKeys(file) {
  const text = readFileSync(file, 'utf8');
  // One file may hold several components, each with its own `t`: a call
  // belongs to the nearest declaration of its name above it.
  const declarations = [...text.matchAll(/const\s+(\w+)\s*=\s*useTranslations\(\s*'([\w.]+)'\s*\)/g)]
    .map((match) => ({ name: match[1], namespace: match[2], at: match.index }));
  const keys = [];
  for (const name of new Set(declarations.map((declaration) => declaration.name))) {
    const call = new RegExp(`(?<![\\w.])${name}\\(\\s*'([\\w.]+)'`, 'g');
    for (const match of text.matchAll(call)) {
      const declaration = declarations
        .filter((candidate) => candidate.name === name && candidate.at < match.index)
        .at(-1);
      if (declaration) keys.push(`${declaration.namespace}.${match[1]}`);
    }
  }
  return keys;
}

describe('message keys', () => {
  test('every literal key the components ask for is in both dictionaries', () => {
    const missing = [];
    for (const file of sources(join(root, 'src'))) {
      for (const key of usedKeys(file)) {
        for (const [language, dictionary] of Object.entries(dictionaries)) {
          if (!has(dictionary, key)) missing.push(`${language}: ${key} (${relative(root, file)})`);
        }
      }
    }
    assert.deepEqual([...new Set(missing)], []);
  });
});

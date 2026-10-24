import assert from 'node:assert/strict';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, test } from 'node:test';

// A failed action has to tell the owner what did not work, in the interface
// language. A native `alert()` cannot: it blocks the window, ignores the theme,
// and the old ones pointed at a developer console the owner does not have.
// Failures go to a toast through `toastFailure`, which keeps the original
// error in the application log.

const root = new URL('../../', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');

// `PermissionsStep.tsx` is not rendered anywhere and goes with the dead-code
// cleanup. `RecordingControls.tsx` is the second half of the same fix: other
// open changes rewrite that file, so its dialogs move to toasts after them.
const ALERT_ALLOWED = new Set([
  'src/components/onboarding/steps/PermissionsStep.tsx',
  'src/components/RecordingControls.tsx',
]);
// The strings those remaining dialogs still show.
const CONSOLE_TEXT_ALLOWED = new Set([
  'recording.initFailedAlert',
  'recording.pauseFailedAlert',
  'recording.resumeFailedAlert',
]);

function sources(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.(tsx?|jsx?)$/.test(name) ? [path] : [];
  });
}

function strings(node, prefix = '') {
  return Object.entries(node).flatMap(([key, value]) =>
    value && typeof value === 'object'
      ? strings(value, `${prefix}${key}.`)
      : [[`${prefix}${key}`, String(value)]],
  );
}

describe('failures are not silent', () => {
  test('no native alert() outside the files still waiting for the fix', () => {
    const found = [];
    for (const file of sources(join(root, 'src'))) {
      const name = relative(root, file).replaceAll('\\', '/');
      if (ALERT_ALLOWED.has(name)) continue;
      const text = readFileSync(file, 'utf8');
      for (const match of text.matchAll(/(?<![\w.])(?:window\.)?alert\(/g)) {
        const line = text.slice(0, match.index).split('\n').length;
        found.push(`${name}:${line}`);
      }
    }
    assert.deepEqual(found, []);
  });

  test('no message sends the owner to a developer console', () => {
    for (const language of ['ru', 'en']) {
      const dictionary = JSON.parse(readFileSync(join(root, `messages/${language}.json`), 'utf8'));
      const found = strings(dictionary)
        .filter(([key, text]) => /консол|console/i.test(text) && !CONSOLE_TEXT_ALLOWED.has(key))
        .map(([key]) => key);
      assert.deepEqual(found, [], language);
    }
  });
});

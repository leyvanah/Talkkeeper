import assert from 'node:assert/strict';
import { describe, mock, test } from 'node:test';
import {
  PRINT_START_TIMEOUT_MS,
  SHEET_POLICY,
  recoverySheetHtml,
  watchPrint,
} from '../../src/lib/recovery-print.ts';

const TEXT = {
  title: 'Код восстановления архива',
  explanation: 'Храните лист отдельно от компьютера.',
  code: 'ABCD-EFGH-JKLM-NPQR',
  issued: 'Выпущен 24.10.2026',
};

describe('recoverySheetHtml', () => {
  test('holds the title, the explanation, the code and the date', () => {
    const html = recoverySheetHtml(TEXT);
    for (const part of Object.values(TEXT)) assert.ok(html.includes(part), part);
  });

  test('forbids the sheet every request', () => {
    const html = recoverySheetHtml(TEXT);
    assert.ok(html.includes(`content="${SHEET_POLICY.replaceAll("'", '&#39;')}"`));
    assert.match(SHEET_POLICY, /default-src 'none'/);
  });

  test('loads nothing from anywhere', () => {
    const html = recoverySheetHtml(TEXT);
    for (const pattern of [/https?:/i, /\ssrc=/i, /<link/i, /<script/i, /url\(/i, /@import/i]) {
      assert.doesNotMatch(html, pattern);
    }
  });

  test('a translation cannot add markup to the sheet', () => {
    const html = recoverySheetHtml({
      ...TEXT,
      explanation: '<img src="http://127.0.0.1:1/x"> "Встреча в четверг" & co',
    });
    assert.doesNotMatch(html, /<img/);
    assert.ok(html.includes('&lt;img src=&quot;http://127.0.0.1:1/x&quot;&gt;'));
    assert.ok(html.includes('&amp; co'));
  });
});

function sheet(print) {
  const listeners = {};
  return {
    listeners,
    addEventListener: (type, listener) => {
      listeners[type] = listener;
    },
    print: () => print(listeners),
  };
}

function scheduler() {
  const pending = [];
  const schedule = mock.fn((ms, run) => {
    const entry = { ms, run, cancelled: false };
    pending.push(entry);
    return () => {
      entry.cancelled = true;
    };
  });
  const runAll = () => pending.filter((entry) => !entry.cancelled).forEach((entry) => entry.run());
  return { schedule, runAll };
}

describe('watchPrint', () => {
  test('a dialog that held print() until it closed is done, with no wait', () => {
    const { schedule } = scheduler();
    const onDone = mock.fn();
    const onFailed = mock.fn();

    watchPrint(
      sheet((on) => {
        on.beforeprint();
        on.afterprint();
      }),
      { onDone, onFailed },
      schedule,
    );

    assert.equal(onDone.mock.callCount(), 1);
    assert.equal(onFailed.mock.callCount(), 0);
    assert.equal(schedule.mock.callCount(), 0);
  });

  test('a dialog that opens after print() returned is not a failure', () => {
    const { schedule, runAll } = scheduler();
    const onDone = mock.fn();
    const onFailed = mock.fn();
    const target = sheet(() => {});

    watchPrint(target, { onDone, onFailed }, schedule);
    target.listeners.beforeprint();
    runAll();
    target.listeners.afterprint();

    assert.equal(schedule.mock.calls[0].arguments[0], PRINT_START_TIMEOUT_MS);
    assert.equal(onFailed.mock.callCount(), 0);
    assert.equal(onDone.mock.callCount(), 1);
  });

  test('print() that returns without a dialog is a failure once the wait is over', () => {
    const { schedule, runAll } = scheduler();
    const onDone = mock.fn();
    const onFailed = mock.fn();

    watchPrint(sheet(() => {}), { onDone, onFailed }, schedule);
    assert.equal(onFailed.mock.callCount(), 0);
    runAll();

    assert.equal(onFailed.mock.callCount(), 1);
    assert.equal(onDone.mock.callCount(), 0);
  });

  test('print() that throws is a failure at once', () => {
    const { schedule } = scheduler();
    const onFailed = mock.fn();
    const error = new Error('no printing here');

    watchPrint(
      sheet(() => {
        throw error;
      }),
      { onDone: mock.fn(), onFailed },
      schedule,
    );

    assert.deepEqual(onFailed.mock.calls[0].arguments, [error]);
    assert.equal(schedule.mock.callCount(), 0);
  });

  test('stopping the wait keeps a closed sheet from reporting a failure', () => {
    const { schedule, runAll } = scheduler();
    const onFailed = mock.fn();

    const stop = watchPrint(sheet(() => {}), { onDone: mock.fn(), onFailed }, schedule);
    stop();
    runAll();

    assert.equal(onFailed.mock.callCount(), 0);
  });
});

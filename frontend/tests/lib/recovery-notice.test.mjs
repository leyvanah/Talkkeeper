import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { NO_RECOVERY_NOTICE, nextRecoveryNotice } from '../../src/lib/recovery-notice.ts';

describe('nextRecoveryNotice', () => {
  test('getting in with the code opens the reminder', () => {
    assert.deepEqual(nextRecoveryNotice(NO_RECOVERY_NOTICE, 'unlockedWithCode'), {
      codeUsed: true,
      open: true,
    });
  });

  test('"later" closes the reminder but remembers the code was used', () => {
    const used = nextRecoveryNotice(NO_RECOVERY_NOTICE, 'unlockedWithCode');
    assert.deepEqual(nextRecoveryNotice(used, 'dismissed'), { codeUsed: true, open: false });
  });

  test('a new code or a deleted one ends the warning', () => {
    const used = nextRecoveryNotice(NO_RECOVERY_NOTICE, 'unlockedWithCode');
    const later = nextRecoveryNotice(used, 'dismissed');
    for (const state of [used, later]) {
      assert.deepEqual(nextRecoveryNotice(state, 'codeReissued'), NO_RECOVERY_NOTICE);
      assert.deepEqual(nextRecoveryNotice(state, 'codeRemoved'), NO_RECOVERY_NOTICE);
    }
  });

  test('closing a reminder that was never shown changes nothing', () => {
    assert.deepEqual(nextRecoveryNotice(NO_RECOVERY_NOTICE, 'dismissed'), NO_RECOVERY_NOTICE);
  });
});

import assert from 'node:assert/strict';
import { beforeEach, describe, test } from 'node:test';
import {
  answerDestination,
  keepNotesDraft,
  notesDraft,
  notesSaved,
  personAiState,
  profileSignature,
  reconcilePersonAi,
  resetPersonPageMemory,
  subscribePersonAi,
  updatePersonAi,
} from '../../src/lib/person-page-memory.ts';

const profile = { displayName: 'Тест А', meetingCount: 2, messageCount: 6, lastSeenAt: '2026-10-01' };

beforeEach(() => resetPersonPageMemory());

describe('overview and answers', () => {
  test('stay for the person after the page is left and opened again', () => {
    reconcilePersonAi('p1', profileSignature(profile));
    updatePersonAi('p1', () => ({ overview: 'Встреча в четверг' }));
    reconcilePersonAi('p1', profileSignature(profile));
    assert.equal(personAiState('p1').overview, 'Встреча в четверг');
    assert.equal(personAiState('p2').overview, null);
  });

  test('a request that ends after leaving still lands for its person', async () => {
    reconcilePersonAi('p1', profileSignature(profile));
    updatePersonAi('p1', () => ({ overviewPending: true }));
    await Promise.resolve();
    updatePersonAi('p1', () => ({ overview: 'готово', overviewPending: false }));
    assert.equal(personAiState('p1').overview, 'готово');
  });

  test('are dropped when the person’s meetings changed', () => {
    reconcilePersonAi('p1', profileSignature(profile));
    updatePersonAi('p1', () => ({ overview: 'старый', history: [{ id: 1, question: 'q', answer: 'a', status: 'complete' }] }));
    reconcilePersonAi('p1', profileSignature({ ...profile, meetingCount: 3 }));
    assert.equal(personAiState('p1').overview, null);
    assert.deepEqual(personAiState('p1').history, []);
  });

  test('one still being written is kept even if the meetings changed', () => {
    reconcilePersonAi('p1', profileSignature(profile));
    updatePersonAi('p1', () => ({ overviewPending: true }));
    reconcilePersonAi('p1', profileSignature({ ...profile, messageCount: 7 }));
    assert.equal(personAiState('p1').overviewPending, true);
  });

  test('the state object is the same until something changes', () => {
    reconcilePersonAi('p1', profileSignature(profile));
    const before = personAiState('p1');
    assert.equal(personAiState('p1'), before);
    let calls = 0;
    const stop = subscribePersonAi(() => { calls += 1; });
    updatePersonAi('p1', () => ({ overview: 'x' }));
    stop();
    assert.notEqual(personAiState('p1'), before);
    assert.equal(calls, 1);
  });
});

describe('notes draft', () => {
  test('waits until the text it holds is saved', () => {
    keepNotesDraft('p1', 'Секретная фраза');
    notesSaved('p1', 'Секретная фраза');
    assert.equal(notesDraft('p1'), undefined);
  });

  test('typing during a save keeps the newer text waiting', () => {
    keepNotesDraft('p1', 'Секрет');
    keepNotesDraft('p1', 'Секретная фраза');
    notesSaved('p1', 'Секрет');
    assert.equal(notesDraft('p1'), 'Секретная фраза');
  });
});

describe('answerDestination', () => {
  const base = { hasModel: true, leavesThisComputer: false, isCloud: false, localOnly: false };

  test('no model chosen', () => {
    assert.equal(answerDestination({ ...base, hasModel: false }), 'none');
  });

  test('on this computer, whatever the mode', () => {
    assert.equal(answerDestination(base), 'here');
    assert.equal(answerDestination({ ...base, localOnly: true }), 'here');
  });

  test('cloud and another machine are told apart', () => {
    assert.equal(answerDestination({ ...base, leavesThisComputer: true, isCloud: true }), 'cloud');
    assert.equal(answerDestination({ ...base, leavesThisComputer: true }), 'remote');
  });

  test('"This computer only" refuses both', () => {
    assert.equal(answerDestination({ ...base, leavesThisComputer: true, isCloud: true, localOnly: true }), 'blocked');
    assert.equal(answerDestination({ ...base, leavesThisComputer: true, localOnly: true }), 'blocked');
  });
});

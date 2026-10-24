import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, test } from 'node:test';
import { retranscriptionText, speakerPassText } from '../../src/lib/rerun-outcome.ts';

const root = new URL('../../', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const dictionaries = ['ru', 'en'].map((lang) => ({
  lang,
  messages: JSON.parse(readFileSync(join(root, `messages/${lang}.json`), 'utf8')).meetingDetails,
}));

/** The keys are chosen at run time, so the key check cannot see them: this does. */
function assertKnown(text) {
  for (const line of [text.title, text.description].filter(Boolean)) {
    for (const { lang, messages } of dictionaries) {
      assert.equal(typeof messages[line.key], 'string', `${lang}.json lacks meetingDetails.${line.key}`);
    }
  }
}

describe('speakerPassText', () => {
  test('a pass that changed nothing says so, and why', () => {
    const text = speakerPassText({ num_speakers: 2, changed: 0, unchanged: 7, kept_by_hand: 0 });
    assert.deepEqual(text, {
      title: { key: 'speakersNothingChanged' },
      description: { key: 'speakersAlreadyLabeled' },
    });
    assertKnown(text);
  });

  test('names a person gave are named as the reason nothing changed', () => {
    const text = speakerPassText({ num_speakers: 2, changed: 0, unchanged: 7, kept_by_hand: 6 });
    assert.equal(text.description.key, 'speakersAlreadyLabeledNamesKept');
    assertKnown(text);
  });

  test('a pass that changed lines counts them, and the ones it kept', () => {
    const text = speakerPassText({ num_speakers: 2, changed: 3, unchanged: 4, kept_by_hand: 2 });
    assert.deepEqual(text, {
      title: { key: 'speakersChanged', values: { count: 3 } },
      description: { key: 'speakersUnchangedWithNames', values: { count: 4, named: 2 } },
    });
    assertKnown(text);
    const plain = speakerPassText({ num_speakers: 2, changed: 3, unchanged: 4, kept_by_hand: 0 });
    assert.equal(plain.description.key, 'speakersUnchangedCount');
    assertKnown(plain);
    assert.equal(speakerPassText({ num_speakers: 2, changed: 5, unchanged: 0, kept_by_hand: 0 }).description, null);
  });

  test('no lines at all: no speakers were told apart', () => {
    const text = speakerPassText({ num_speakers: 0, changed: 0, unchanged: 0, kept_by_hand: 0 });
    assert.equal(text.title.key, 'noSpeakersDetected');
    assertKnown(text);
  });
});

describe('retranscriptionText', () => {
  test('names the new lines and the corrected ones left as they were', () => {
    const text = retranscriptionText({ segments_count: 1, kept_count: 6 });
    assert.deepEqual(text, {
      title: { key: 'retranscribeDone', values: { count: 1 } },
      description: { key: 'retranscribeKeptEdited', values: { count: 6 } },
    });
    assertKnown(text);
  });

  test('without corrections there is nothing kept to mention', () => {
    assert.equal(retranscriptionText({ segments_count: 7, kept_count: 0 }).description, null);
    // An older backend does not report it.
    assert.equal(retranscriptionText({ segments_count: 7 }).description, null);
  });
});

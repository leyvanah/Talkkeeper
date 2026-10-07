import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { speakerKeyForTerm, speakerLabel } from '../../src/lib/speaker-label.ts';

const ru = {
  you: 'Вы',
  youWithName: (name) => `${name} (Вы)`,
  guest: 'Гость',
  numbered: (number) => `Спикер ${number}`,
};

describe('speakerLabel', () => {
  test('the capture labels are shown in the interface language', () => {
    assert.equal(speakerLabel('Guest', '', ru), 'Гость');
    assert.equal(speakerLabel('Speaker 2', '', ru), 'Спикер 2');
    assert.equal(speakerLabel('speaker 12', '', ru), 'Спикер 12');
  });

  test('the owner is "<name> (Вы)" on screen and just the name outside', () => {
    assert.equal(speakerLabel('You', 'Тест', ru), 'Тест (Вы)');
    assert.equal(speakerLabel('You', 'Тест', ru, 'export'), 'Тест');
    // No name set: one word either way.
    assert.equal(speakerLabel('You', '  ', ru), 'Вы');
    assert.equal(speakerLabel('You', '', ru, 'export'), 'Вы');
  });

  test('a name someone gave a speaker is shown as given', () => {
    assert.equal(speakerLabel('Анна', 'Тест', ru), 'Анна');
    assert.equal(speakerLabel('Speaker Facilitator', '', ru), 'Speaker Facilitator');
  });

  test('a line said by both names each voice', () => {
    assert.equal(speakerLabel('You + Speaker 1', 'Тест', ru), 'Тест (Вы) + Спикер 1');
    assert.equal(speakerLabel('You + Guest', 'Тест', ru, 'export'), 'Тест + Гость');
  });
});

describe('speakerKeyForTerm', () => {
  test('the words on screen lead back to the stored keys', () => {
    assert.equal(speakerKeyForTerm('вы', ru), 'You');
    assert.equal(speakerKeyForTerm(' Гость ', ru), 'Guest');
    assert.equal(speakerKeyForTerm('спикер 3', ru), 'Speaker 3');
  });

  test('anything else is searched as typed', () => {
    assert.equal(speakerKeyForTerm('смета', ru), null);
    assert.equal(speakerKeyForTerm('спикер', ru), null);
    assert.equal(speakerKeyForTerm('', ru), null);
  });
});

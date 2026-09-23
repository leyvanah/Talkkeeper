import test from 'node:test'
import assert from 'node:assert/strict'
import { splitPoint } from '../../src/lib/transcript-split.ts'

const cut = (text, cursor) => {
  const at = splitPoint(text, cursor)
  return [text.slice(0, at).trim(), text.slice(at).trim()]
}

test('a cursor between words cuts there', () => {
  assert.deepEqual(cut('раз два три', 7), ['раз два', 'три'])
})

test('a cursor inside a word gives the whole word to the second half', () => {
  const text = 'Бралам Карлам-Барлам-бар-лам. И вот'
  assert.deepEqual(cut(text, text.indexOf('р-лам.')), ['Бралам', 'Карлам-Барлам-бар-лам. И вот'])
  assert.deepEqual(cut('раз два три', 6), ['раз', 'два три'])
})

test('inside the first word the cut moves to its end', () => {
  assert.deepEqual(cut('первое второе', 3), ['первое', 'второе'])
})

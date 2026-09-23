import test from 'node:test'
import assert from 'node:assert/strict'
import { speakerChoices } from '../../src/lib/transcript-speakers.ts'

test('every voice of a meeting is offered once, the local user first', () => {
  const choices = speakerChoices(['Speaker 1', 'You', 'You + Speaker 1', 'Анна', undefined, 'speaker 1'])
  assert.deepEqual(choices.existing, ['You', 'Speaker 1', 'Анна'])
})

test('the local user is offered even before they have said anything', () => {
  assert.deepEqual(speakerChoices(['Guest']).existing, ['You', 'Guest'])
})

test('a new speaker takes the next free number', () => {
  assert.equal(speakerChoices(['You', 'Speaker 1', 'Speaker 3']).fresh, 'Speaker 4')
  assert.equal(speakerChoices(['You', 'Анна']).fresh, 'Speaker 1')
})

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { chooseEnhancementModel } from '../../src/lib/enhancement-model.ts';

const gigaam = { provider: 'gigaam', name: 'gigaam-v3-e2e-rnnt-int8' };
const parakeet = { provider: 'parakeet', name: 'parakeet-tdt-0.6b-v3-int8' };
const whisperSmall = { provider: 'whisper', name: 'small' };
const whisperLarge = { provider: 'whisper', name: 'large-v3' };
const external = { provider: 'externalStt', name: 'Service' };

const liveGigaam = { provider: 'gigaam', model: gigaam.name };

describe('chooseEnhancementModel', () => {
  test('uses the post-call choice when its model is on disk', () => {
    const result = chooseEnhancementModel(
      [gigaam, parakeet],
      { provider: 'parakeet', model: parakeet.name },
      liveGigaam,
    );
    assert.deepEqual(result, { model: parakeet, replaced: null });
  });

  test('follows the live model when post-call is left on live', () => {
    const result = chooseEnhancementModel([parakeet, gigaam], { provider: 'live', model: '' }, liveGigaam);
    assert.deepEqual(result, { model: gigaam, replaced: null });
  });

  test('takes GigaAM as a post-call choice', () => {
    const result = chooseEnhancementModel(
      [parakeet, gigaam],
      { provider: 'gigaam', model: gigaam.name },
      { provider: 'parakeet', model: parakeet.name },
    );
    assert.deepEqual(result, { model: gigaam, replaced: null });
  });

  test('keeps the kind of model when the exact one was removed', () => {
    const result = chooseEnhancementModel(
      [whisperSmall, gigaam],
      { provider: 'whisper', model: 'large-v3' },
      liveGigaam,
    );
    assert.deepEqual(result, { model: whisperSmall, replaced: null });
  });

  test('falls back to the live model when the post-call model is missing', () => {
    const result = chooseEnhancementModel(
      [whisperLarge, gigaam],
      { provider: 'parakeet', model: parakeet.name },
      liveGigaam,
    );
    assert.deepEqual(result, { model: gigaam, replaced: 'parakeet' });
  });

  test('never falls back to the external service', () => {
    const result = chooseEnhancementModel(
      [external, gigaam],
      { provider: 'parakeet', model: parakeet.name },
      { provider: 'externalStt', model: external.name },
    );
    assert.deepEqual(result, { model: gigaam, replaced: 'parakeet' });
  });

  test('uses the external service when it is the chosen model', () => {
    const result = chooseEnhancementModel(
      [gigaam, external],
      { provider: 'live', model: '' },
      { provider: 'externalStt', model: external.name },
    );
    assert.deepEqual(result, { model: external, replaced: null });
  });

  test('keeps the live kind of model, and otherwise prefers Parakeet', () => {
    const result = chooseEnhancementModel(
      [whisperSmall, gigaam, parakeet],
      { provider: 'live', model: '' },
      { provider: 'localWhisper', model: 'large-v3-turbo' },
    );
    assert.deepEqual(result, { model: whisperSmall, replaced: null });

    const noWhisper = chooseEnhancementModel(
      [gigaam, parakeet],
      { provider: 'live', model: '' },
      { provider: 'localWhisper', model: 'large-v3-turbo' },
    );
    assert.deepEqual(noWhisper, { model: parakeet, replaced: 'whisper' });
  });

  test('returns nothing when no local model is on disk', () => {
    const result = chooseEnhancementModel(
      [external],
      { provider: 'parakeet', model: parakeet.name },
      liveGigaam,
    );
    assert.equal(result, null);
  });
});

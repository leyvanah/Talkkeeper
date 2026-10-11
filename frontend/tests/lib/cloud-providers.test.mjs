import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import {
  CLOUD_SUMMARY_PROVIDERS,
  LOCAL_SUMMARY_PROVIDERS,
  isCloudProvider,
  isLoopbackUrl,
  sendsOffThisComputer,
} from '../../src/lib/cloud-providers.ts';

// The same cases as network_policy.rs, so the window and the backend agree on
// what "this computer" means.
describe('isLoopbackUrl', () => {
  test('this computer is recognised in every spelling', () => {
    for (const url of [
      'http://localhost:11434',
      'http://LOCALHOST:8080/v1',
      'http://127.0.0.1:1234',
      'http://127.1.2.3',
      'http://[::1]:8000/v1',
      '  http://localhost:11434  ',
    ]) {
      assert.equal(isLoopbackUrl(url), true, url);
    }
  });

  test('anything else is not', () => {
    for (const url of [
      'https://api.openai.com/v1/models',
      'http://192.168.1.10:11434',
      'http://localhost.example.com',
      'http://0.0.0.0:11434',
      'not a url',
      '',
    ]) {
      assert.equal(isLoopbackUrl(url), false, url);
    }
  });
});

describe('isCloudProvider', () => {
  test('the four cloud services are cloud, the rest are not', () => {
    for (const provider of CLOUD_SUMMARY_PROVIDERS) assert.equal(isCloudProvider(provider), true);
    for (const provider of LOCAL_SUMMARY_PROVIDERS) assert.equal(isCloudProvider(provider), false);
  });
});

describe('sendsOffThisComputer', () => {
  test('a cloud provider always does', () => {
    assert.equal(sendsOffThisComputer('openai', 'http://localhost:1234'), true);
  });

  test('the built-in model never does', () => {
    assert.equal(sendsOffThisComputer('builtin-ai', 'https://example.com'), false);
  });

  test('Ollama goes where its address points, localhost when none is set', () => {
    assert.equal(sendsOffThisComputer('ollama', ''), false);
    assert.equal(sendsOffThisComputer('ollama', null), false);
    assert.equal(sendsOffThisComputer('ollama', 'http://127.0.0.1:11434'), false);
    assert.equal(sendsOffThisComputer('ollama', 'http://192.168.1.10:11434'), true);
  });

  test('a custom server goes where its address points', () => {
    assert.equal(sendsOffThisComputer('custom-openai', 'http://localhost:8000/v1'), false);
    assert.equal(sendsOffThisComputer('custom-openai', 'https://llm.example.com/v1'), true);
    assert.equal(sendsOffThisComputer('custom-openai', ''), false);
  });
});

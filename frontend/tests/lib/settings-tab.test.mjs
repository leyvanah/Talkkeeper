import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import {
  pickSettingsTab,
  rememberSettingsTab,
  rememberedSettingsTab,
} from '../../src/lib/settings-tab.ts';

const TABS = ['general', 'recording', 'security', 'about'];

describe('pickSettingsTab', () => {
  test('opens on the remembered tab', () => {
    assert.equal(pickSettingsTab(null, 'security', TABS, 'general'), 'security');
  });

  test('a tab another screen asked for wins over the remembered one', () => {
    assert.equal(pickSettingsTab('recording', 'security', TABS, 'general'), 'recording');
  });

  test('nothing remembered opens on the fallback', () => {
    assert.equal(pickSettingsTab(null, null, TABS, 'general'), 'general');
  });

  test('a name that is not a tab is ignored', () => {
    assert.equal(pickSettingsTab('gone', 'removed', TABS, 'general'), 'general');
    assert.equal(pickSettingsTab('gone', 'about', TABS, 'general'), 'about');
  });
});

describe('rememberSettingsTab', () => {
  test('keeps the last tab for the next time the page mounts', () => {
    rememberSettingsTab('security');
    rememberSettingsTab('about');
    assert.equal(rememberedSettingsTab(), 'about');
  });
});

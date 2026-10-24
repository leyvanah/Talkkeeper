import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { isTemplateDraftEmpty } from '../../src/lib/template-draft.ts';

const section = (patch = {}) => ({ title: '', instruction: '', format: 'list', ...patch });
const draft = (patch = {}) => ({ name: '', description: '', sections: [section()], ...patch });

describe('isTemplateDraftEmpty', () => {
  test('a fresh form is empty', () => {
    assert.equal(isTemplateDraftEmpty(draft()), true);
  });

  test('spaces alone, a changed format and extra blank sections lose nothing', () => {
    assert.equal(
      isTemplateDraftEmpty(draft({
        name: '  ',
        sections: [section({ format: 'table' }), section(), section({ item_format: ' ' })],
      })),
      true,
    );
  });

  test('a typed name or description is something to lose', () => {
    assert.equal(isTemplateDraftEmpty(draft({ name: 'Встреча в четверг' })), false);
    assert.equal(isTemplateDraftEmpty(draft({ description: 'Итоги' })), false);
  });

  test('text in any section is something to lose', () => {
    assert.equal(
      isTemplateDraftEmpty(draft({ sections: [section(), section({ instruction: 'Кратко' })] })),
      false,
    );
    assert.equal(isTemplateDraftEmpty(draft({ sections: [section({ title: 'Решения' })] })), false);
    assert.equal(isTemplateDraftEmpty(draft({ sections: [section({ item_format: '- {item}' })] })), false);
  });
});

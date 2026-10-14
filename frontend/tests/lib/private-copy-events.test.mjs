import assert from 'node:assert/strict';
import { describe, mock, test } from 'node:test';
import { handleClipboardEvent, selectionToCopy } from '../../src/lib/private-copy-events.ts';

function environment({ field = null, selected = null, editable = false } = {}) {
  return {
    focusedField: () => field,
    selectedText: () => selected,
    selectionIsEditable: () => editable,
    deleteSelection: mock.fn(),
  };
}

function clipboardEvent(type = 'copy') {
  return { type, preventDefault: mock.fn(), stopImmediatePropagation: mock.fn() };
}

const textarea = (value, start, end, extra = {}) => ({
  type: 'textarea',
  value,
  selectionStart: start,
  selectionEnd: end,
  ...extra,
});

describe('selectionToCopy', () => {
  test('takes the selection on the page', () => {
    assert.equal(selectionToCopy(environment({ selected: 'Встреча в четверг' })), 'Встреча в четверг');
  });

  test('takes the selected part of a focused field', () => {
    const field = textarea('Секретная фраза', 0, 9);
    assert.equal(selectionToCopy(environment({ field, selected: 'другое' })), 'Секретная');
  });

  test('a password field gives nothing', () => {
    const field = { type: 'password', value: 'Секретная фраза', selectionStart: 0, selectionEnd: 15 };
    assert.equal(selectionToCopy(environment({ field })), null);
  });

  test('a field without a selection API gives nothing', () => {
    const field = { type: 'number', value: '15', selectionStart: null, selectionEnd: null };
    assert.equal(selectionToCopy(environment({ field, selected: 'другое' })), null);
  });

  test('a focused field with nothing selected wins over the page selection', () => {
    const field = textarea('Секретная фраза', 4, 4);
    assert.equal(selectionToCopy(environment({ field, selected: 'другое' })), null);
  });
});

describe('handleClipboardEvent', () => {
  test('a selection is copied privately and the browser copy is stopped', () => {
    const event = clipboardEvent();
    const copy = mock.fn(async () => {});

    handleClipboardEvent(event, environment({ selected: 'Встреча в четверг' }), copy, () => {});

    assert.deepEqual(copy.mock.calls[0].arguments, ['Встреча в четверг']);
    assert.equal(event.preventDefault.mock.callCount(), 1);
    assert.equal(event.stopImmediatePropagation.mock.callCount(), 1);
  });

  test('nothing selected — the event is left alone', () => {
    const event = clipboardEvent();
    const copy = mock.fn(async () => {});

    handleClipboardEvent(event, environment(), copy, () => {});

    assert.equal(copy.mock.callCount(), 0);
    assert.equal(event.preventDefault.mock.callCount(), 0);
    assert.equal(event.stopImmediatePropagation.mock.callCount(), 0);
  });

  test('a failed write is reported', async () => {
    const onError = mock.fn();
    const copy = async () => {
      throw 'OpenClipboard failed: access denied';
    };

    handleClipboardEvent(clipboardEvent(), environment({ selected: 'текст' }), copy, onError);
    await new Promise((resolve) => setImmediate(resolve));

    assert.deepEqual(onError.mock.calls[0].arguments, ['OpenClipboard failed: access denied']);
  });

  test('a cut removes the selection where it can be edited', () => {
    const env = environment({ selected: 'текст', editable: true });

    handleClipboardEvent(clipboardEvent('cut'), env, async () => {}, () => {});

    assert.equal(env.deleteSelection.mock.callCount(), 1);
  });

  test('a cut of text that cannot be edited does nothing, the browser included', () => {
    const copy = mock.fn(async () => {});
    const page = environment({ selected: 'текст', editable: false });
    const readOnlyField = environment({ field: textarea('текст', 0, 5, { readOnly: true }) });

    for (const env of [page, readOnlyField]) {
      const event = clipboardEvent('cut');
      handleClipboardEvent(event, env, copy, () => {});
      assert.equal(event.preventDefault.mock.callCount(), 1);
      assert.equal(env.deleteSelection.mock.callCount(), 0);
    }
    assert.equal(copy.mock.callCount(), 0);
  });

  test('a selection without text is not copied the browser way either', () => {
    const event = clipboardEvent();
    const copy = mock.fn(async () => {});

    handleClipboardEvent(event, environment({ selected: '' }), copy, () => {});

    assert.equal(event.preventDefault.mock.callCount(), 1);
    assert.equal(event.stopImmediatePropagation.mock.callCount(), 1);
    assert.equal(copy.mock.callCount(), 0);
  });

  test('a copy never deletes', () => {
    const env = environment({ selected: 'текст', editable: true });

    handleClipboardEvent(clipboardEvent('copy'), env, async () => {}, () => {});

    assert.equal(env.deleteSelection.mock.callCount(), 0);
  });
});

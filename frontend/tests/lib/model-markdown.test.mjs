import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { addressToShow, modelMarkdownComponents } from '../../src/lib/model-markdown.ts';

function render(markdown) {
  return renderToStaticMarkup(
    createElement(ReactMarkdown, { remarkPlugins: [remarkGfm], components: modelMarkdownComponents }, markdown),
  );
}

describe('model answer links', () => {
  test('a link is text with its full address, not something to click', () => {
    const html = render('[Открыть подробности](http://127.0.0.1:8765/leak?q=Sekretnaya-fraza)');

    assert.doesNotMatch(html, /<a[\s>]/);
    assert.doesNotMatch(html, /href=/);
    assert.match(html, /Открыть подробности/);
    assert.match(html, /http:\/\/127\.0\.0\.1:8765\/leak\?q=Sekretnaya-fraza/);
  });

  test('a bare address is shown once', () => {
    const html = render('См. https://example.com/page');

    assert.doesNotMatch(html, /<a[\s>]/);
    assert.equal(html.split('https://example.com/page').length - 1, 1);
  });

  test('a reference-style and an autolink are not clickable either', () => {
    const html = render('[Встреча в четверг][1] и <https://example.com/x>\n\n[1]: https://example.com/y');

    assert.doesNotMatch(html, /<a[\s>]/);
    assert.match(html, /https:\/\/example\.com\/y/);
    assert.match(html, /https:\/\/example\.com\/x/);
  });

  test('an image is not fetched: its caption and address are text', () => {
    const html = render('![Секретная фраза](http://127.0.0.1:8765/img.png)');

    assert.doesNotMatch(html, /<img[\s>]/);
    assert.doesNotMatch(html, /src=/);
    assert.match(html, /Секретная фраза/);
    assert.match(html, /http:\/\/127\.0\.0\.1:8765\/img\.png/);
  });

  test('an image without a caption still shows where it pointed', () => {
    const html = render('![](http://127.0.0.1:8765/img.png)');

    assert.doesNotMatch(html, /<img[\s>]/);
    assert.match(html, /http:\/\/127\.0\.0\.1:8765\/img\.png/);
  });

  test('a footnote does not show its in-page address', () => {
    const html = render('Встреча в четверг[^1]\n\n[^1]: Секретная фраза');

    assert.doesNotMatch(html, /<a[\s>]/);
    assert.doesNotMatch(html, /#user-content-/);
    assert.match(html, /Секретная фраза/);
  });

  test('ordinary markdown still renders', () => {
    const html = render('**Итог**\n\n- первый пункт');

    assert.match(html, /<strong>Итог<\/strong>/);
    assert.match(html, /<li>первый пункт<\/li>/);
  });
});

describe('addressToShow', () => {
  test('nothing to add without an address or when the text is the address', () => {
    assert.equal(addressToShow('текст', undefined), null);
    assert.equal(addressToShow('текст', '  '), null);
    assert.equal(addressToShow(' https://example.com ', 'https://example.com'), null);
    assert.equal(addressToShow('1', '#user-content-fn-1'), null);
  });

  test('otherwise the address as written', () => {
    assert.equal(addressToShow('подробнее', 'https://example.com/a?b=c'), 'https://example.com/a?b=c');
  });
});

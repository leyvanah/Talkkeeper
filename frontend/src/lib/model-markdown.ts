/**
 * How model answers render their links and images.
 *
 * A model writes its answer from what it was given (the transcript, notes,
 * a person's profile), so any address in it may carry that text. A plain
 * `<a href>` would load the address in the app window itself, past the
 * local-only network policy; a plain `<img>` would fetch it as soon as the
 * answer shows. Both are rendered as text instead: what the model wrote and
 * the full address, which can be read and copied but goes nowhere.
 */

import { createElement, type ReactNode } from 'react';
import type { Components } from 'react-markdown';

interface HastNode {
  type: string;
  value?: string;
  children?: HastNode[];
}

/** The plain text inside a markdown element, as the reader sees it. */
export function textOf(node: HastNode | undefined): string {
  if (!node) return '';
  if (node.type === 'text') return node.value ?? '';
  return (node.children ?? []).map(textOf).join('');
}

/**
 * The address to show after the visible text, or null when it adds nothing:
 * there is none, it only points within the answer (a footnote), or the text
 * already is the address (a bare link).
 */
export function addressToShow(text: string, href: string | undefined | null): string | null {
  const address = (href ?? '').trim();
  if (!address || address.startsWith('#')) return null;
  if (text.trim() === address) return null;
  return address;
}

function address(value: string): ReactNode {
  return createElement('span', { className: 'break-all text-muted-foreground' }, value);
}

export const modelMarkdownComponents: Components = {
  a({ node, href, children }) {
    const shown = addressToShow(textOf(node as HastNode | undefined), href);
    if (!shown) return createElement('span', { className: 'break-all' }, children);
    return createElement('span', null, children, ' (', address(shown), ')');
  },
  img({ alt, src }) {
    const label = (alt ?? '').trim();
    const shown = typeof src === 'string' ? addressToShow(label, src) : null;
    if (!shown) return label ? createElement('span', null, label) : null;
    if (!label) return address(shown);
    return createElement('span', null, label, ' (', address(shown), ')');
  },
};

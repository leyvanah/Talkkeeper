'use client';

/**
 * The strip along the top of the page: what is open on the left, the window's
 * own buttons on the right. The window has no system frame, so this row is
 * also what the window is dragged and maximized by.
 *
 * Because the buttons sit in the row rather than over the page, nothing below
 * has to leave a corner free for them.
 */

import React, { createContext, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft } from 'lucide-react';
import { WindowControls } from './WindowControls';

type Back = { label: string; onBack: () => void } | null;

const WindowTitleContext = createContext<{
  title: string;
  setTitle: (title: string | null) => void;
  back: Back;
  setBack: (back: Back) => void;
} | null>(null);

export function WindowTitleProvider({ children }: { children: React.ReactNode }) {
  const [title, setTitle] = useState<string | null>(null);
  const [back, setBack] = useState<Back>(null);
  const value = useMemo(() => ({ title: title ?? 'Talkkeeper', setTitle, back, setBack }), [title, back]);
  return <WindowTitleContext.Provider value={value}>{children}</WindowTitleContext.Provider>;
}

/**
 * Puts a way back in the header, so a page does not repeat it. Kept in a ref
 * so that a handler written inline does not re-register on every render.
 */
export function useWindowBack(label: string, onBack: (() => void) | null) {
  const context = useContext(WindowTitleContext);
  const setBack = context?.setBack;
  const handler = useRef(onBack);
  handler.current = onBack;

  useEffect(() => {
    if (!setBack) return;
    if (!onBack) {
      setBack(null);
      return;
    }
    setBack({ label, onBack: () => handler.current?.() });
    return () => setBack(null);
    // The handler itself lives in the ref above.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [setBack, label, !onBack]);
}

/**
 * Names what is open, in the header and in the window's own title — which is
 * what the taskbar and Alt-Tab show. Cleared when the page goes away.
 */
export function useWindowTitle(title: string | null | undefined) {
  const context = useContext(WindowTitleContext);
  const setTitle = context?.setTitle;

  useEffect(() => {
    if (!setTitle) return;
    setTitle(title ?? null);
    return () => setTitle(null);
  }, [setTitle, title]);

  useEffect(() => {
    let cancelled = false;
    import('@tauri-apps/api/window')
      .then(({ getCurrentWindow }) => {
        if (cancelled) return;
        return getCurrentWindow().setTitle(title ? `${title} — Talkkeeper` : 'Talkkeeper');
      })
      .catch(() => {
        // Outside Tauri there is no window to name.
      });
    return () => {
      cancelled = true;
    };
  }, [title]);
}

export function AppHeader() {
  const context = useContext(WindowTitleContext);
  const title = context?.title ?? 'Talkkeeper';
  const back = context?.back ?? null;

  return (
    <div
      data-tauri-drag-region="deep"
      className="flex h-10 shrink-0 items-center gap-2 border-b border-[var(--af-border)] bg-[var(--af-panel)] pl-2 pr-2"
    >
      {back ? (
        <button
          type="button"
          onClick={back.onBack}
          title={back.label}
          aria-label={back.label}
          className="flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-sm text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)]"
        >
          <ArrowLeft size={15} />
          <span className="hidden sm:inline">{back.label}</span>
        </button>
      ) : (
        <span className="w-2" />
      )}
      <span className="min-w-0 flex-1 truncate text-sm font-medium text-[var(--af-text)]">
        {title}
      </span>
      <WindowControls />
    </div>
  );
}

'use client';

/**
 * The strip along the top of the page: what is open on the left, the window's
 * own buttons on the right. The window has no system frame, so this row is
 * also what the window is dragged and maximized by.
 *
 * Because the buttons sit in the row rather than over the page, nothing below
 * has to leave a corner free for them.
 */

import React, { createContext, useContext, useEffect, useMemo, useState } from 'react';
import { WindowControls } from './WindowControls';

const WindowTitleContext = createContext<{
  title: string;
  setTitle: (title: string | null) => void;
} | null>(null);

export function WindowTitleProvider({ children }: { children: React.ReactNode }) {
  const [title, setTitle] = useState<string | null>(null);
  const value = useMemo(() => ({ title: title ?? 'Talkkeeper', setTitle }), [title]);
  return <WindowTitleContext.Provider value={value}>{children}</WindowTitleContext.Provider>;
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
  const title = useContext(WindowTitleContext)?.title ?? 'Talkkeeper';

  return (
    <div
      data-tauri-drag-region="deep"
      className="flex h-10 shrink-0 items-center gap-3 border-b border-[var(--af-border)] bg-[var(--af-panel)] pl-4 pr-2"
    >
      <span className="min-w-0 flex-1 truncate text-sm font-medium text-[var(--af-text-2)]">
        {title}
      </span>
      <WindowControls />
    </div>
  );
}

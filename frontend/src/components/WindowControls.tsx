'use client';

/**
 * The window's own buttons, since the window has no system frame of its own.
 * They sit in the corner of the interface rather than on a bar of their own.
 *
 * Closing hides the window to the tray, the way the system button did: the
 * recording keeps running. The tray menu has the real quit.
 */

import React, { useEffect, useState } from 'react';
import { useTranslations } from 'next-intl';
import { Minus, Square, Copy, X } from 'lucide-react';
import { getCurrentWindow } from '@tauri-apps/api/window';

/** Tauri's unlisten is asynchronous: a failure in it arrives as a rejected
 *  promise that try/catch would let through, over the whole window. */
function quietly(unlisten: (() => void) | undefined) {
  try {
    void Promise.resolve(unlisten?.() as unknown).catch(() => {});
  } catch {
    // Already gone.
  }
}

function useWindowMaximized(): [boolean, () => void] {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;

    const follow = async () => {
      try {
        // Inside the try: before Tauri is there (a browser preview, the very
        // first frame) this throws, and the buttons should simply do nothing.
        const appWindow = getCurrentWindow();
        const now = await appWindow.isMaximized();
        if (!cancelled) setMaximized(now);
        const unlisten = await appWindow.onResized(async () => {
          try {
            setMaximized(await appWindow.isMaximized());
          } catch {
            // The window is going away; nothing to report.
          }
        });
        if (cancelled) quietly(unlisten);
        else stop = unlisten;
      } catch {
        // Outside Tauri (a browser preview), the buttons simply do nothing.
      }
    };
    follow();

    return () => {
      cancelled = true;
      quietly(stop);
    };
  }, []);

  return [maximized, () => void getCurrentWindow().toggleMaximize().catch(() => {})];
}

const button =
  'flex h-7 w-8 items-center justify-center rounded-md text-[var(--af-text-3)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)]';

export function WindowControls({ className = '' }: { className?: string }) {
  const t = useTranslations('window');
  const [maximized, toggleMaximize] = useWindowMaximized();

  return (
    <div className={`flex shrink-0 items-center gap-0.5 ${className}`}>
      <button
        type="button"
        onClick={() => void getCurrentWindow().minimize().catch(() => {})}
        title={t('minimize')}
        aria-label={t('minimize')}
        className={button}
      >
        <Minus size={14} />
      </button>
      <button
        type="button"
        onClick={toggleMaximize}
        title={maximized ? t('restore') : t('maximize')}
        aria-label={maximized ? t('restore') : t('maximize')}
        className={button}
      >
        {maximized ? <Copy size={12} /> : <Square size={12} />}
      </button>
      <button
        type="button"
        onClick={() => void getCurrentWindow().close().catch(() => {})}
        title={t('close')}
        aria-label={t('close')}
        className={`${button} hover:bg-[#c42b1c] hover:text-white`}
      >
        <X size={15} />
      </button>
    </div>
  );
}

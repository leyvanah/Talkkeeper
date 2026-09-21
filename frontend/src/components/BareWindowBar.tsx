'use client';

/**
 * The window's buttons for the screens that stand in for the app: the lock
 * screen, the first-run setup, a failed start, a crash report.
 *
 * The window has no system frame, and the app's own header lives inside the
 * app — so these screens had no way to be closed, moved or minimised at all.
 * This is the same row without a title: something to drag the window by, and
 * the three buttons in the corner.
 */

import { WindowControls } from './WindowControls';

export function BareWindowBar() {
  return (
    <div
      data-tauri-drag-region="deep"
      className="fixed inset-x-0 top-0 z-[60] flex h-10 items-center justify-end pr-2"
    >
      <WindowControls />
    </div>
  );
}

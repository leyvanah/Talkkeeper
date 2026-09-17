'use client';

/**
 * Edges to resize the window by. A window without a system frame keeps no
 * resize border of its own, so the app draws thin invisible ones.
 *
 * They sit above the page but only along its outer few pixels, and each one
 * hands the drag straight to the window manager.
 */

import React from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';

type Direction =
  | 'North'
  | 'South'
  | 'East'
  | 'West'
  | 'NorthEast'
  | 'NorthWest'
  | 'SouthEast'
  | 'SouthWest';

const EDGES: { direction: Direction; className: string }[] = [
  { direction: 'North', className: 'top-0 left-2 right-2 h-1 cursor-ns-resize' },
  { direction: 'South', className: 'bottom-0 left-2 right-2 h-1 cursor-ns-resize' },
  { direction: 'West', className: 'left-0 top-2 bottom-2 w-1 cursor-ew-resize' },
  { direction: 'East', className: 'right-0 top-2 bottom-2 w-1 cursor-ew-resize' },
  { direction: 'NorthWest', className: 'top-0 left-0 h-2 w-2 cursor-nwse-resize' },
  { direction: 'NorthEast', className: 'top-0 right-0 h-2 w-2 cursor-nesw-resize' },
  { direction: 'SouthWest', className: 'bottom-0 left-0 h-2 w-2 cursor-nesw-resize' },
  { direction: 'SouthEast', className: 'bottom-0 right-0 h-2 w-2 cursor-nwse-resize' },
];

export function WindowResizeEdges() {
  return (
    <>
      {EDGES.map(({ direction, className }) => (
        <div
          key={direction}
          aria-hidden
          onPointerDown={(event) => {
            if (event.button !== 0) return;
            event.preventDefault();
            void getCurrentWindow()
              .startResizeDragging(direction)
              .catch(() => {});
          }}
          className={`fixed z-[60] ${className}`}
        />
      ))}
    </>
  );
}

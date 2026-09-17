'use client';

/**
 * A vertical divider that can be dragged sideways, or moved with the arrow
 * keys once focused. Double-click puts it back where it started out.
 *
 * While dragging it only reports the pointer: the owner previews the new
 * size on the DOM and commits it when the drag ends, so a drag does not
 * re-render the panels on every move.
 */

import React, { useRef } from 'react';
import { setPanelResizing } from './PanelLayoutProvider';

interface ResizeHandleProps {
  label: string;
  /** Current position, for assistive technology, as a percentage or pixels. */
  valueNow: number;
  valueMin: number;
  valueMax: number;
  onDrag: (clientX: number) => void;
  onDragEnd: (clientX: number) => void;
  /** An arrow key was pressed; -1 is left. Shift makes the step bigger. */
  onStep: (direction: -1 | 1, big: boolean) => void;
  onReset: () => void;
  className?: string;
  style?: React.CSSProperties;
}

export function ResizeHandle({
  label,
  valueNow,
  valueMin,
  valueMax,
  onDrag,
  onDragEnd,
  onStep,
  onReset,
  className = '',
  style,
}: ResizeHandleProps) {
  const dragging = useRef(false);

  const finish = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging.current) return;
    dragging.current = false;
    setPanelResizing(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    onDragEnd(event.clientX);
  };

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      title={label}
      aria-valuenow={Math.round(valueNow)}
      aria-valuemin={Math.round(valueMin)}
      aria-valuemax={Math.round(valueMax)}
      tabIndex={0}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        dragging.current = true;
        event.currentTarget.setPointerCapture(event.pointerId);
        setPanelResizing(true);
      }}
      onPointerMove={(event) => {
        if (dragging.current) onDrag(event.clientX);
      }}
      onPointerUp={finish}
      onPointerCancel={finish}
      onDoubleClick={onReset}
      onKeyDown={(event) => {
        if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
          event.preventDefault();
          onStep(event.key === 'ArrowLeft' ? -1 : 1, event.shiftKey);
        } else if (event.key === 'Home' || event.key === 'Enter') {
          event.preventDefault();
          onReset();
        }
      }}
      className={`group/handle z-20 flex w-2 shrink-0 cursor-col-resize touch-none justify-center outline-none ${className}`}
      style={style}
    >
      <div className="h-full w-px bg-[var(--af-border)] transition-colors group-hover/handle:w-0.5 group-hover/handle:bg-[var(--af-accent)] group-focus-visible/handle:w-0.5 group-focus-visible/handle:bg-[var(--af-accent)]" />
    </div>
  );
}

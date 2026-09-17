'use client';

/**
 * The layout of the resizable panels: the sidebar's width and whether it is
 * collapsed, and how the meeting page shares its width. Remembered between
 * launches.
 *
 * The sidebar's room is published as the `--sidebar-offset` CSS variable, so
 * anything positioned next to it follows a drag without re-rendering.
 */

import React, { createContext, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useState } from 'react';
import {
  DEFAULT_LAYOUT,
  MeetingPanes,
  PanelLayout,
  clampSidebarWidth,
  clampSplit,
  loadLayout,
  saveLayout,
  sidebarOffset,
  togglePane,
} from '@/lib/panel-layout';

// The app is prerendered; a layout effect there only warns.
const useIsomorphicLayoutEffect = typeof window === 'undefined' ? useEffect : useLayoutEffect;

export function setSidebarOffsetVariable(px: number): void {
  document.documentElement.style.setProperty('--sidebar-offset', `${px}px`);
}

/** Mark a drag in progress, which turns off animations and text selection. */
export function setPanelResizing(on: boolean): void {
  if (on) document.documentElement.setAttribute('data-panel-resizing', '');
  else document.documentElement.removeAttribute('data-panel-resizing');
}

interface PanelLayoutContextType {
  layout: PanelLayout;
  toggleSidebar: () => void;
  collapseSidebar: (collapsed: boolean) => void;
  setSidebarWidth: (width: number) => void;
  setSplit: (split: number) => void;
  togglePane: (pane: 'transcript' | 'summary') => void;
  setPanes: (panes: MeetingPanes) => void;
}

const PanelLayoutContext = createContext<PanelLayoutContextType | null>(null);

export function usePanelLayout(): PanelLayoutContextType {
  const context = useContext(PanelLayoutContext);
  if (!context) throw new Error('usePanelLayout must be used within a PanelLayoutProvider');
  return context;
}

export function PanelLayoutProvider({ children }: { children: React.ReactNode }) {
  const [layout, setLayout] = useState<PanelLayout>(DEFAULT_LAYOUT);
  const [loaded, setLoaded] = useState(false);

  // Read before the first paint, so a remembered layout does not jump.
  useIsomorphicLayoutEffect(() => {
    setLayout(loadLayout());
    setLoaded(true);
  }, []);

  useIsomorphicLayoutEffect(() => {
    setSidebarOffsetVariable(sidebarOffset(layout));
  }, [layout.sidebarCollapsed, layout.sidebarWidth]);

  useEffect(() => {
    if (loaded) saveLayout(layout);
  }, [layout, loaded]);

  const update = useCallback((change: (current: PanelLayout) => Partial<PanelLayout>) => {
    setLayout((current) => {
      const next = { ...current, ...change(current) };
      return (Object.keys(next) as (keyof PanelLayout)[]).every((k) => next[k] === current[k]) ? current : next;
    });
  }, []);

  const value = useMemo<PanelLayoutContextType>(
    () => ({
      layout,
      toggleSidebar: () => update((c) => ({ sidebarCollapsed: !c.sidebarCollapsed })),
      collapseSidebar: (collapsed) => update(() => ({ sidebarCollapsed: collapsed })),
      setSidebarWidth: (width) => update(() => ({ sidebarWidth: clampSidebarWidth(width), sidebarCollapsed: false })),
      setSplit: (split) => update(() => ({ split: clampSplit(split) })),
      togglePane: (pane) => update((c) => ({ panes: togglePane(c.panes, pane) })),
      setPanes: (panes) => update(() => ({ panes })),
    }),
    [layout, update]
  );

  return <PanelLayoutContext.Provider value={value}>{children}</PanelLayoutContext.Provider>;
}

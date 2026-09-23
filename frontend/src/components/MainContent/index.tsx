'use client';

import React from 'react';
import { AppHeader } from '@/components/AppHeader';

interface MainContentProps {
  children: React.ReactNode;
}

const MainContent: React.FC<MainContentProps> = ({ children }) => {
  return (
    // min-w-0 is required: flex items default to min-width:auto and will not
    // shrink below their content, which clipped Settings (and other pages)
    // when the window was narrower than sidebar + content.
    <main
      className="flex flex-1 min-w-0 min-h-0 h-screen flex-col overflow-hidden transition-[margin] duration-300"
      style={{ marginLeft: 'var(--sidebar-offset)' }}
    >
      <AppHeader />
      {/* No gutter of its own: every page brings its own padding, and a strip
          of page background between the sidebar and the content read as a
          black bar in the dark themes. */}
      <div className="min-w-0 min-h-0 flex-1 overflow-hidden">
        {children}
      </div>
    </main>
  );
};

export default MainContent;

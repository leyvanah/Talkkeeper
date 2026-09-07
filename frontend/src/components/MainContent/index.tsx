'use client';

import React from 'react';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { RetranscriptionIndicator } from '@/components/shared/RetranscriptionIndicator';

interface MainContentProps {
  children: React.ReactNode;
}

const MainContent: React.FC<MainContentProps> = ({ children }) => {
  const { isCollapsed } = useSidebar();

  return (
    // min-w-0 is required: flex items default to min-width:auto and will not
    // shrink below their content, which clipped Settings (and other pages)
    // when the window was narrower than sidebar + content.
    <main
      className={`flex flex-1 min-w-0 min-h-0 h-screen flex-col overflow-hidden transition-all duration-300 ${
        isCollapsed ? 'ml-16' : 'ml-64'
      }`}
    >
      <div className="min-w-0 min-h-0 flex-1 overflow-hidden pl-4 sm:pl-6 lg:pl-8">
        {children}
      </div>
      {/* Background work reports here rather than over the page. It belongs to
          this column and not to the window, because the sidebar is fixed and
          full-height: a strip across the window would run underneath it. */}
      <RetranscriptionIndicator />
    </main>
  );
};

export default MainContent;

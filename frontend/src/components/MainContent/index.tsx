'use client';

import React from 'react';
import { SIDEBAR_COLLAPSED_WIDTH, useSidebar } from '@/components/Sidebar/SidebarProvider';

interface MainContentProps {
  children: React.ReactNode;
}

const MainContent: React.FC<MainContentProps> = ({ children }) => {
  const { isCollapsed, sidebarWidth, isResizingSidebar } = useSidebar();

  return (
    <main
      className={`flex-1 min-w-0 overflow-hidden ${isResizingSidebar ? '' : 'transition-[margin] duration-300'}`}
      style={{ marginLeft: isCollapsed ? SIDEBAR_COLLAPSED_WIDTH : sidebarWidth }}
    >
      <div className="pl-8 min-w-0 w-full max-w-full overflow-hidden">
        {children}
      </div>
    </main>
  );
};

export default MainContent;

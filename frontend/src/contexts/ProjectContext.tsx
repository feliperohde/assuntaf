'use client';

import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import {
  DEFAULT_PROJECT_ID,
  Project,
  getActiveProjectId,
  projectService,
  storeActiveProjectId,
} from '@/services/projectService';

interface ProjectContextType {
  projects: Project[];
  activeProjectId: string;
  activeProject: Project | null;
  setActiveProjectId: (projectId: string) => void;
  refreshProjects: () => Promise<void>;
}

const ProjectContext = createContext<ProjectContextType | null>(null);

export const useProject = () => {
  const context = useContext(ProjectContext);
  if (!context) {
    throw new Error('useProject must be used within a ProjectProvider');
  }
  return context;
};

export function ProjectProvider({ children }: { children: React.ReactNode }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeProjectId, setActiveProjectIdState] = useState<string>(DEFAULT_PROJECT_ID);

  const setActiveProjectId = useCallback((projectId: string) => {
    storeActiveProjectId(projectId);
    setActiveProjectIdState(projectId);
  }, []);

  const refreshProjects = useCallback(async () => {
    try {
      const list = await projectService.listProjects();
      setProjects(list);
      // Fall back to the default project if the stored one was deleted or archived
      const stored = getActiveProjectId();
      const next = list.some(p => p.id === stored) ? stored : DEFAULT_PROJECT_ID;
      setActiveProjectId(next);
    } catch (error) {
      console.error('Failed to load projects:', error);
    }
  }, [setActiveProjectId]);

  useEffect(() => {
    refreshProjects();
  }, [refreshProjects]);

  const activeProject = useMemo(
    () => projects.find(p => p.id === activeProjectId) ?? null,
    [projects, activeProjectId]
  );

  return (
    <ProjectContext.Provider
      value={{ projects, activeProjectId, activeProject, setActiveProjectId, refreshProjects }}
    >
      {children}
    </ProjectContext.Provider>
  );
}

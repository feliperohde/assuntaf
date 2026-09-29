/**
 * Project Service
 *
 * Wraps the project Tauri commands. Every meeting belongs to a project; the active
 * project scopes the meeting list, search, new recordings, and imports.
 */

import { invoke } from '@tauri-apps/api/core';

export const DEFAULT_PROJECT_ID = 'project-default';
const ACTIVE_PROJECT_KEY = 'activeProjectId';

export interface Project {
  id: string;
  name: string;
  description: string | null;
  contextMd: string | null;
  glossary: string | null;
  ticketPatterns: string | null;
  color: string | null;
  archived: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ProjectInput {
  name: string;
  description?: string | null;
  contextMd?: string | null;
  glossary?: string | null;
  ticketPatterns?: string | null;
  color?: string | null;
  archived?: boolean;
}

export interface ProjectMember {
  id: string;
  projectId: string;
  name: string;
  role: string | null;
  email: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ProjectMemberInput {
  name: string;
  role?: string | null;
  email?: string | null;
}

/** Active project id persisted across sessions; falls back to the default project. */
export function getActiveProjectId(): string {
  try {
    return localStorage.getItem(ACTIVE_PROJECT_KEY) || DEFAULT_PROJECT_ID;
  } catch {
    return DEFAULT_PROJECT_ID;
  }
}

export function storeActiveProjectId(projectId: string): void {
  try {
    localStorage.setItem(ACTIVE_PROJECT_KEY, projectId);
  } catch {
    // Storage unavailable: the selection just won't persist
  }
}

export const projectService = {
  listProjects: (includeArchived = false) =>
    invoke<Project[]>('list_projects', { includeArchived }),
  getProject: (projectId: string) =>
    invoke<Project | null>('get_project', { projectId }),
  createProject: (project: ProjectInput) =>
    invoke<Project>('create_project', { project }),
  updateProject: (projectId: string, project: ProjectInput) =>
    invoke<Project>('update_project', { projectId, project }),
  deleteProject: (projectId: string) =>
    invoke<boolean>('delete_project', { projectId }),
  setMeetingProject: (meetingId: string, projectId: string) =>
    invoke<boolean>('set_meeting_project', { meetingId, projectId }),
  listMembers: (projectId: string) =>
    invoke<ProjectMember[]>('list_project_members', { projectId }),
  createMember: (projectId: string, member: ProjectMemberInput) =>
    invoke<ProjectMember>('create_project_member', { projectId, member }),
  updateMember: (memberId: string, member: ProjectMemberInput) =>
    invoke<ProjectMember>('update_project_member', { memberId, member }),
  deleteMember: (memberId: string) =>
    invoke<boolean>('delete_project_member', { memberId }),
};

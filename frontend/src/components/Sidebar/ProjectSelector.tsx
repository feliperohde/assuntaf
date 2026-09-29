'use client';

import React from 'react';
import { useRouter } from 'next/navigation';
import { FolderKanban, Settings2 } from 'lucide-react';
import { useProject } from '@/contexts/ProjectContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';

/** Switches the active project. Locked while recording so the meeting lands in the project it started in. */
export function ProjectSelector() {
  const router = useRouter();
  const { projects, activeProjectId, setActiveProjectId } = useProject();
  const { isRecording } = useRecordingState();

  return (
    <div className="flex items-center gap-1 mb-2">
      <Select value={activeProjectId} onValueChange={setActiveProjectId} disabled={isRecording}>
        <SelectTrigger className="h-9 flex-1 min-w-0" title={isRecording ? 'Cannot switch projects while recording' : 'Active project'}>
          <FolderKanban className="w-4 h-4 mr-2 flex-shrink-0 text-gray-600" />
          <SelectValue placeholder="Project" />
        </SelectTrigger>
        <SelectContent>
          {projects.map(project => (
            <SelectItem key={project.id} value={project.id}>
              <span className="flex items-center gap-2">
                <span
                  className="inline-block w-2 h-2 rounded-full"
                  style={{ backgroundColor: project.color || '#9ca3af' }}
                />
                {project.name}
              </span>
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <button
        onClick={() => router.push('/projects')}
        className="p-2 rounded-md hover:bg-gray-100 flex-shrink-0"
        aria-label="Manage projects"
        title="Manage projects"
      >
        <Settings2 className="w-4 h-4 text-gray-600" />
      </button>
    </div>
  );
}

'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { ArrowLeft, Plus, Trash2, UserPlus, Check, FolderKanban } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { Label } from '@/components/ui/label';
import { ConfirmationModal } from '@/components/ConfirmationModel/confirmation-modal';
import { useProject } from '@/contexts/ProjectContext';
import {
  DEFAULT_PROJECT_ID,
  Project,
  ProjectInput,
  ProjectMember,
  projectService,
} from '@/services/projectService';

const COLORS = ['#3b82f6', '#10b981', '#f59e0b', '#ef4444', '#8b5cf6', '#ec4899', '#14b8a6', '#6b7280'];

const EMPTY_FORM: ProjectInput = {
  name: '',
  description: '',
  contextMd: '',
  glossary: '',
  ticketPatterns: '',
  color: COLORS[0],
  archived: false,
};

function toForm(project: Project): ProjectInput {
  return {
    name: project.name,
    description: project.description ?? '',
    contextMd: project.contextMd ?? '',
    glossary: project.glossary ?? '',
    ticketPatterns: project.ticketPatterns ?? '',
    color: project.color ?? COLORS[0],
    archived: project.archived,
  };
}

/** Converts empty strings to null so optional fields are stored as NULL. */
function normalize(form: ProjectInput): ProjectInput {
  const clean = (v?: string | null) => (v && v.trim() ? v.trim() : null);
  return {
    name: form.name.trim(),
    description: clean(form.description),
    contextMd: clean(form.contextMd),
    glossary: clean(form.glossary),
    ticketPatterns: clean(form.ticketPatterns),
    color: form.color ?? null,
    archived: !!form.archived,
  };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export default function ProjectsPage() {
  const router = useRouter();
  const { activeProjectId, setActiveProjectId, refreshProjects } = useProject();

  const [projects, setProjects] = useState<Project[]>([]);
  // null = creating a new project
  const [selectedId, setSelectedId] = useState<string | null>(activeProjectId);
  const [form, setForm] = useState<ProjectInput>(EMPTY_FORM);
  const [members, setMembers] = useState<ProjectMember[]>([]);
  const [newMember, setNewMember] = useState({ name: '', role: '' });
  const [saving, setSaving] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const loadProjects = useCallback(async () => {
    try {
      setProjects(await projectService.listProjects(true));
    } catch (error) {
      toast.error('Failed to load projects', { description: errorMessage(error) });
    }
  }, []);

  useEffect(() => {
    loadProjects();
  }, [loadProjects]);

  // Sync the form and member list with the selected project
  useEffect(() => {
    if (selectedId === null) {
      setForm(EMPTY_FORM);
      setMembers([]);
      return;
    }
    const project = projects.find(p => p.id === selectedId);
    if (!project) return;
    setForm(toForm(project));
    projectService
      .listMembers(selectedId)
      .then(setMembers)
      .catch(error => toast.error('Failed to load members', { description: errorMessage(error) }));
  }, [selectedId, projects]);

  const updateField = <K extends keyof ProjectInput>(key: K, value: ProjectInput[K]) =>
    setForm(prev => ({ ...prev, [key]: value }));

  const handleSave = async () => {
    if (!form.name.trim()) {
      toast.error('Project name is required');
      return;
    }
    setSaving(true);
    try {
      const saved = selectedId === null
        ? await projectService.createProject(normalize(form))
        : await projectService.updateProject(selectedId, normalize(form));
      await loadProjects();
      await refreshProjects();
      setSelectedId(saved.id);
      toast.success(selectedId === null ? 'Project created' : 'Project saved');
    } catch (error) {
      toast.error('Failed to save project', { description: errorMessage(error) });
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    setConfirmDelete(false);
    if (!selectedId) return;
    try {
      await projectService.deleteProject(selectedId);
      await loadProjects();
      await refreshProjects();
      setSelectedId(DEFAULT_PROJECT_ID);
      toast.success('Project deleted', { description: 'Its meetings were moved to the default project' });
    } catch (error) {
      toast.error('Failed to delete project', { description: errorMessage(error) });
    }
  };

  const handleAddMember = async () => {
    if (!selectedId || !newMember.name.trim()) return;
    try {
      const member = await projectService.createMember(selectedId, {
        name: newMember.name.trim(),
        role: newMember.role.trim() || null,
      });
      setMembers(prev => [...prev, member].sort((a, b) => a.name.localeCompare(b.name)));
      setNewMember({ name: '', role: '' });
    } catch (error) {
      toast.error('Failed to add member', { description: errorMessage(error) });
    }
  };

  const handleRemoveMember = async (memberId: string) => {
    try {
      await projectService.deleteMember(memberId);
      setMembers(prev => prev.filter(m => m.id !== memberId));
    } catch (error) {
      toast.error('Failed to remove member', { description: errorMessage(error) });
    }
  };

  const isDefault = selectedId === DEFAULT_PROJECT_ID;

  return (
    <div className="h-screen bg-gray-50 flex flex-col">
      <div className="sticky top-0 z-10 bg-gray-50 border-b border-gray-200">
        <div className="max-w-6xl mx-auto px-8 py-6">
          <div className="flex items-center gap-4">
            <button
              onClick={() => router.back()}
              className="flex items-center gap-2 text-gray-600 hover:text-gray-900 transition-colors"
            >
              <ArrowLeft className="w-5 h-5" />
              <span>Back</span>
            </button>
            <h1 className="text-3xl font-bold">Projects</h1>
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        <div className="max-w-6xl mx-auto p-8 pt-6 grid grid-cols-1 md:grid-cols-[240px_1fr] gap-6">
          {/* Project list */}
          <div className="space-y-1">
            <Button variant="outline" className="w-full justify-start mb-2" onClick={() => setSelectedId(null)}>
              <Plus className="w-4 h-4 mr-2" /> New project
            </Button>
            {projects.map(project => (
              <button
                key={project.id}
                onClick={() => setSelectedId(project.id)}
                className={`w-full flex items-center gap-2 px-3 py-2 rounded-md text-sm text-left transition-colors ${
                  selectedId === project.id ? 'bg-blue-100 text-blue-700 font-medium' : 'hover:bg-gray-100'
                } ${project.archived ? 'opacity-60' : ''}`}
              >
                <span className="w-2.5 h-2.5 rounded-full flex-shrink-0" style={{ backgroundColor: project.color || '#9ca3af' }} />
                <span className="flex-1 truncate">{project.name}</span>
                {project.id === activeProjectId && <Check className="w-4 h-4 text-blue-600" aria-label="Active project" />}
                {project.archived && <span className="text-xs text-gray-500">archived</span>}
              </button>
            ))}
          </div>

          {/* Editor */}
          <div className="bg-white rounded-lg border border-gray-200 p-6 space-y-5">
            <div className="flex items-center justify-between">
              <h2 className="text-xl font-semibold flex items-center gap-2">
                <FolderKanban className="w-5 h-5 text-gray-600" />
                {selectedId === null ? 'New project' : form.name || 'Project'}
              </h2>
              {selectedId && selectedId !== activeProjectId && !form.archived && (
                <Button variant="outline" size="sm" onClick={() => setActiveProjectId(selectedId)}>
                  Set as active
                </Button>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor="project-name">Name</Label>
              <Input id="project-name" value={form.name} onChange={e => updateField('name', e.target.value)} placeholder="e.g. Payments platform" />
            </div>

            <div className="space-y-2">
              <Label>Color</Label>
              <div className="flex gap-2">
                {COLORS.map(color => (
                  <button
                    key={color}
                    onClick={() => updateField('color', color)}
                    className={`w-6 h-6 rounded-full border-2 ${form.color === color ? 'border-gray-900' : 'border-transparent'}`}
                    style={{ backgroundColor: color }}
                    aria-label={`Color ${color}`}
                  />
                ))}
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="project-description">Description</Label>
              <Input id="project-description" value={form.description ?? ''} onChange={e => updateField('description', e.target.value)} placeholder="One-line summary" />
            </div>

            <div className="space-y-2">
              <Label htmlFor="project-context">Context</Label>
              <Textarea
                id="project-context"
                rows={5}
                value={form.contextMd ?? ''}
                onChange={e => updateField('contextMd', e.target.value)}
                placeholder="Goals, stack, team, conventions… Used as background for summaries and questions about this project."
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="project-glossary">Glossary</Label>
              <Textarea
                id="project-glossary"
                rows={3}
                value={form.glossary ?? ''}
                onChange={e => updateField('glossary', e.target.value)}
                placeholder="Names, acronyms and jargon, one per line (helps transcription and search)"
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="project-tickets">Ticket ID patterns</Label>
              <Input
                id="project-tickets"
                value={form.ticketPatterns ?? ''}
                onChange={e => updateField('ticketPatterns', e.target.value)}
                placeholder="e.g. PAY-\d+, OPS-\d+"
              />
            </div>

            {selectedId && !isDefault && (
              <label className="flex items-center gap-2 text-sm text-gray-700">
                <input type="checkbox" checked={!!form.archived} onChange={e => updateField('archived', e.target.checked)} />
                Archived (hidden from the project selector)
              </label>
            )}

            <div className="flex items-center gap-2 pt-2">
              <Button variant="blue" onClick={handleSave} disabled={saving}>
                {selectedId === null ? 'Create project' : 'Save'}
              </Button>
              {selectedId && !isDefault && (
                <Button variant="outline" className="text-red-600" onClick={() => setConfirmDelete(true)}>
                  <Trash2 className="w-4 h-4 mr-2" /> Delete
                </Button>
              )}
            </div>

            {/* Members */}
            {selectedId && (
              <div className="border-t border-gray-100 pt-5 space-y-3">
                <h3 className="font-semibold">Members</h3>
                <p className="text-sm text-gray-500">People who take part in this project's meetings. Used to identify speakers.</p>
                <ul className="divide-y divide-gray-100">
                  {members.map(member => (
                    <li key={member.id} className="flex items-center justify-between py-2 text-sm">
                      <span>
                        <span className="font-medium">{member.name}</span>
                        {member.role && <span className="text-gray-500"> · {member.role}</span>}
                      </span>
                      <button
                        onClick={() => handleRemoveMember(member.id)}
                        className="p-1 rounded-md hover:bg-red-50 hover:text-red-600"
                        aria-label={`Remove ${member.name}`}
                      >
                        <Trash2 className="w-4 h-4" />
                      </button>
                    </li>
                  ))}
                  {members.length === 0 && <li className="py-2 text-sm text-gray-400">No members yet</li>}
                </ul>
                <div className="flex gap-2">
                  <Input
                    value={newMember.name}
                    onChange={e => setNewMember(prev => ({ ...prev, name: e.target.value }))}
                    onKeyDown={e => e.key === 'Enter' && handleAddMember()}
                    placeholder="Name"
                  />
                  <Input
                    value={newMember.role}
                    onChange={e => setNewMember(prev => ({ ...prev, role: e.target.value }))}
                    onKeyDown={e => e.key === 'Enter' && handleAddMember()}
                    placeholder="Role (optional)"
                  />
                  <Button variant="outline" onClick={handleAddMember} disabled={!newMember.name.trim()}>
                    <UserPlus className="w-4 h-4" />
                  </Button>
                </div>
              </div>
            )}
          </div>
        </div>
      </div>

      <ConfirmationModal
        isOpen={confirmDelete}
        text="Delete this project? Its meetings will be moved to the default project."
        onConfirm={handleDelete}
        onCancel={() => setConfirmDelete(false)}
      />
    </div>
  );
}

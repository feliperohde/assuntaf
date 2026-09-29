'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Users, ChevronDown, ChevronRight, Loader2, ScanFace } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { DIARIZATION_EVENT, MeetingSpeaker, diarizationService } from '@/services/diarizationService';
import { ProjectMember, projectService } from '@/services/projectService';

const UNLINKED = '__none__';

function formatDuration(seconds: number) {
  const m = Math.floor(seconds / 60);
  const s = Math.round(seconds % 60);
  return m > 0 ? `${m}m ${s}s` : `${s}s`;
}

/**
 * Who spoke in this meeting. Speakers are detected from the recording; linking a
 * speaker to a project member names them in the transcript and teaches their
 * voice so they are recognized automatically in later meetings.
 */
export function SpeakersPanel({ meetingId, onSpeakersChanged }: { meetingId: string; onSpeakersChanged?: () => void }) {
  const [open, setOpen] = useState(false);
  const [speakers, setSpeakers] = useState<MeetingSpeaker[]>([]);
  const [members, setMembers] = useState<ProjectMember[]>([]);
  const [detecting, setDetecting] = useState(false);
  const [editing, setEditing] = useState<Record<string, string>>({});

  const load = useCallback(async () => {
    try {
      setSpeakers(await diarizationService.listSpeakers(meetingId));
      const meeting = await invoke<{ project_id?: string | null }>('api_get_meeting', { meetingId });
      if (meeting?.project_id) {
        setMembers(await projectService.listMembers(meeting.project_id));
      }
    } catch (error) {
      console.error('Failed to load speakers:', error);
    }
  }, [meetingId]);

  useEffect(() => {
    load();
  }, [load]);

  // Automatic detection runs in the background after a meeting is saved
  useEffect(() => {
    const unlisten = listen<{ meetingId: string }>(DIARIZATION_EVENT, event => {
      if (event.payload.meetingId === meetingId) {
        load();
        onSpeakersChanged?.();
      }
    });
    return () => {
      unlisten.then(fn => fn());
    };
  }, [meetingId, load, onSpeakersChanged]);

  const detect = async () => {
    setDetecting(true);
    try {
      const outcome = await diarizationService.diarize(meetingId);
      setSpeakers(outcome.speakers);
      toast.success(`Detected ${outcome.speakers.length} speaker(s)`, {
        description: `${outcome.assignedSegments} of ${outcome.totalSegments} transcript lines attributed.`,
      });
      onSpeakersChanged?.();
    } catch (error) {
      toast.error('Speaker detection failed', { description: String(error) });
    } finally {
      setDetecting(false);
    }
  };

  const update = async (speaker: MeetingSpeaker, label: string | null, memberId: string | null) => {
    try {
      setSpeakers(await diarizationService.updateSpeaker(speaker.id, label, memberId));
      onSpeakersChanged?.();
    } catch (error) {
      toast.error('Failed to update speaker', { description: String(error) });
    }
  };

  return (
    <div className="border-b border-gray-200">
      <button
        onClick={() => setOpen(!open)}
        className="w-full flex items-center gap-2 px-4 py-2 text-sm text-gray-700 hover:bg-gray-50"
      >
        {open ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
        <Users className="w-4 h-4" />
        <span className="font-medium">Speakers</span>
        <span className="text-gray-400">{speakers.length > 0 ? `(${speakers.length})` : '— not detected'}</span>
      </button>

      {open && (
        <div className="px-4 pb-3 space-y-2">
          {speakers.map(speaker => (
            <div key={speaker.id} className="flex items-center gap-2">
              <Input
                className="h-8 w-40"
                value={editing[speaker.id] ?? speaker.label}
                onChange={e => setEditing(prev => ({ ...prev, [speaker.id]: e.target.value }))}
                onBlur={() => {
                  const label = editing[speaker.id];
                  if (label !== undefined && label.trim() && label !== speaker.label) {
                    update(speaker, label, speaker.memberId);
                  }
                }}
                title="Speaker label"
              />
              <Select
                value={speaker.memberId ?? UNLINKED}
                onValueChange={value => update(speaker, null, value === UNLINKED ? null : value)}
              >
                <SelectTrigger className="h-8 flex-1">
                  <SelectValue placeholder="Link to member" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNLINKED}>Not a project member</SelectItem>
                  {members.map(member => (
                    <SelectItem key={member.id} value={member.id}>
                      {member.name}
                      {member.role ? ` · ${member.role}` : ''}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <span className="text-xs text-gray-500 w-16 text-right">{formatDuration(speaker.speakingSeconds)}</span>
            </div>
          ))}

          {members.length === 0 && speakers.length > 0 && (
            <p className="text-xs text-gray-500">Add members on the Projects page to link speakers to people.</p>
          )}

          <Button variant="outline" size="sm" onClick={detect} disabled={detecting}>
            {detecting ? <Loader2 className="w-4 h-4 mr-2 animate-spin" /> : <ScanFace className="w-4 h-4 mr-2" />}
            {detecting ? 'Detecting…' : speakers.length > 0 ? 'Detect again' : 'Detect speakers'}
          </Button>
          <p className="text-xs text-gray-500">
            Runs locally on the recording. The first run downloads the speaker models (~35 MB).
            Linking a speaker to a member lets later meetings recognize their voice.
          </p>
        </div>
      )}
    </div>
  );
}

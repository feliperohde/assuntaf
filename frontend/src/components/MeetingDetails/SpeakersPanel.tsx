'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Users, ChevronDown, ChevronRight, Loader2, ScanFace, Sparkles, Check, AudioLines } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { DIARIZATION_EVENT, MeetingSpeaker, diarizationService } from '@/services/diarizationService';
import { ProjectMember, projectService } from '@/services/projectService';
import { useI18n } from '@/i18n';

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
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [speakers, setSpeakers] = useState<MeetingSpeaker[]>([]);
  const [members, setMembers] = useState<ProjectMember[]>([]);
  const [detecting, setDetecting] = useState(false);
  const [naming, setNaming] = useState(false);
  const [peopleCount, setPeopleCount] = useState('');
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
      const count = parseInt(peopleCount, 10);
      const outcome = await diarizationService.diarize(meetingId, Number.isFinite(count) && count > 0 ? count : null);
      setSpeakers(outcome.speakers);
      const extras = [
        outcome.namedSpeakers > 0 && t('speakers.namedFromTranscript', { count: outcome.namedSpeakers }),
        outcome.mergedSpeakers > 0 && t('speakers.duplicatesMerged', { count: outcome.mergedSpeakers }),
      ].filter(Boolean);
      toast.success(t('speakers.detected', { count: outcome.speakers.length }), {
        description: [
          t('speakers.linesAttributed', { assigned: outcome.assignedSegments, total: outcome.totalSegments }),
          extras.join(', '),
          outcome.namingError && t('speakers.namingError', { error: outcome.namingError }),
        ]
          .filter(Boolean)
          .join(' '),
      });
      onSpeakersChanged?.();
    } catch (error) {
      toast.error(t('speakers.detectFailed'), { description: String(error) });
    } finally {
      setDetecting(false);
    }
  };

  const inferNames = async () => {
    setNaming(true);
    try {
      const outcome = await diarizationService.inferNames(meetingId);
      await load();
      toast.success(
        outcome.named + outcome.merged > 0
          ? outcome.merged ? t('speakers.namedMerged', { count: outcome.named, merged: outcome.merged }) : t('speakers.named', { count: outcome.named })
          : t('speakers.noNames'),
        { description: t('speakers.namesHelp') }
      );
      if (outcome.named + outcome.merged > 0) onSpeakersChanged?.();
    } catch (error) {
      toast.error(t('speakers.inferFailed'), { description: String(error) });
    } finally {
      setNaming(false);
    }
  };

  const merge = async (into: MeetingSpeaker, from: MeetingSpeaker) => {
    try {
      setSpeakers(await diarizationService.mergeSpeakers(into.id, from.id));
      toast.success(t('speakers.merged', { from: from.memberName ?? from.label, into: into.memberName ?? into.label }));
      onSpeakersChanged?.();
    } catch (error) {
      toast.error(t('speakers.mergeFailed'), { description: String(error) });
    }
  };

  const update = async (speaker: MeetingSpeaker, label: string | null, memberId: string | null) => {
    try {
      setSpeakers(await diarizationService.updateSpeaker(speaker.id, label, memberId));
      onSpeakersChanged?.();
    } catch (error) {
      toast.error(t('speakers.updateFailed'), { description: String(error) });
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
        <span className="font-medium">{t('speakers.title')}</span>
        <span className="text-gray-400">{speakers.length > 0 ? `(${speakers.length})` : t('speakers.notDetected')}</span>
      </button>

      {open && (
        <div className="px-4 pb-3 space-y-2">
          {speakers.map(speaker => (
            <div key={speaker.id} className="space-y-1 pb-1">
              <div className="flex flex-wrap items-center gap-2">
                <Input
                  className="h-8 flex-1 min-w-[8rem]"
                  value={editing[speaker.id] ?? speaker.label}
                  onChange={e => setEditing(prev => ({ ...prev, [speaker.id]: e.target.value }))}
                  onBlur={() => {
                    const label = editing[speaker.id];
                    if (label !== undefined && label.trim() && label !== speaker.label) {
                      update(speaker, label, speaker.memberId);
                    }
                  }}
                  title={t('speakers.label')}
                />
                <Select
                  value={speaker.memberId ?? UNLINKED}
                  onValueChange={value => update(speaker, null, value === UNLINKED ? null : value)}
                >
                  <SelectTrigger className="h-8 flex-1 min-w-[9rem]">
                    <SelectValue placeholder={t('speakers.linkMember')} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={UNLINKED}>{t('speakers.notMember')}</SelectItem>
                    {members.map(member => (
                      <SelectItem key={member.id} value={member.id}>
                        {member.name}
                        {member.role ? ` · ${member.role}` : ''}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {speakers.length > 1 && (
                  <select
                    aria-label={t('speakers.mergeAria', { name: speaker.label })}
                    title={t('speakers.mergeTitle')}
                    value=""
                    onChange={e => {
                      const into = speakers.find(s => s.id === e.target.value);
                      if (into) merge(into, speaker);
                    }}
                    className="h-8 w-24 rounded-md border border-gray-200 bg-white px-1 text-xs text-gray-600"
                  >
                    <option value="">{t('speakers.mergeInto')}</option>
                    {speakers
                      .filter(other => other.id !== speaker.id)
                      .map(other => (
                        <option key={other.id} value={other.id}>
                          {other.memberName ?? other.label}
                        </option>
                      ))}
                  </select>
                )}
                <span className="text-xs text-gray-500 whitespace-nowrap">{formatDuration(speaker.speakingSeconds)}</span>
              </div>
              {speaker.nameSource === 'inferred' && (
                <div className="flex items-center gap-2 pl-1 text-xs text-violet-700">
                  <Sparkles className="w-3 h-3 flex-shrink-0" />
                  <span className="truncate" title={speaker.nameEvidence ?? undefined}>
                    {t('speakers.guessed')}{speaker.nameEvidence ? `: “${speaker.nameEvidence}”` : ''}
                  </span>
                  <button
                    onClick={() => update(speaker, speaker.label, speaker.memberId)}
                    className="ml-auto flex items-center gap-1 rounded px-1.5 py-0.5 hover:bg-violet-50"
                    title={speaker.memberId ? t('speakers.confirmMember') : t('speakers.confirmName')}
                  >
                    <Check className="w-3 h-3" /> {t('speakers.confirm')}
                  </button>
                </div>
              )}
              {speaker.nameSource === 'voice' && (
                <div className="flex items-center gap-1 pl-1 text-xs text-gray-500">
                  <AudioLines className="w-3 h-3" /> {t('speakers.byVoice')}
                </div>
              )}
            </div>
          ))}

          {members.length === 0 && speakers.length > 0 && (
            <p className="text-xs text-gray-500">{t('speakers.addMembers')}</p>
          )}

          <div className="flex flex-wrap items-center gap-2 pt-1">
            <label className="flex items-center gap-1.5 text-xs text-gray-600" title={t('speakers.peopleCountHint')}>
              {t('speakers.peopleCount')}
              <Input
                type="number"
                min={1}
                max={20}
                placeholder={t('speakers.auto')}
                value={peopleCount}
                onChange={e => setPeopleCount(e.target.value)}
                className="h-8 w-16"
              />
            </label>
            <Button variant="outline" size="sm" onClick={detect} disabled={detecting || naming}>
              {detecting ? <Loader2 className="w-4 h-4 mr-2 animate-spin" /> : <ScanFace className="w-4 h-4 mr-2" />}
              {detecting ? t('speakers.detecting') : speakers.length > 0 ? t('speakers.detectAgain') : t('speakers.detect')}
            </Button>
            {speakers.length > 0 && (
              <Button variant="outline" size="sm" onClick={inferNames} disabled={detecting || naming}>
                {naming ? <Loader2 className="w-4 h-4 mr-2 animate-spin" /> : <Sparkles className="w-4 h-4 mr-2" />}
                {t('speakers.guessNames')}
              </Button>
            )}
          </div>
          <p className="text-xs text-gray-500">
            {t('speakers.help')}
          </p>
        </div>
      )}
    </div>
  );
}

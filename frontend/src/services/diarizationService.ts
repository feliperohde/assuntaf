/**
 * Diarization Service
 *
 * Speaker detection for recorded meetings and linking speakers to project members.
 */

import { invoke } from '@tauri-apps/api/core';

export interface MeetingSpeaker {
  id: string;
  label: string;
  memberId: string | null;
  memberName: string | null;
  speakingSeconds: number;
  /** 'voice' (recognized), 'inferred' (guessed from the transcript), 'manual'; null = unnamed. */
  nameSource: 'voice' | 'inferred' | 'manual' | null;
  /** Quote supporting an inferred name. */
  nameEvidence: string | null;
}

export interface DiarizeOutcome {
  meetingId: string;
  speakers: MeetingSpeaker[];
  assignedSegments: number;
  totalSegments: number;
  namedSpeakers: number;
  mergedSpeakers: number;
  namingError: string | null;
}

export interface NamingOutcome {
  named: number;
  merged: number;
}

export const DIARIZATION_EVENT = 'diarization-complete';

export const diarizationService = {
  diarize: (meetingId: string, numSpeakers?: number | null) =>
    invoke<DiarizeOutcome>('diarize_meeting', { meetingId, numSpeakers: numSpeakers ?? null }),
  inferNames: (meetingId: string) => invoke<NamingOutcome>('infer_speaker_names', { meetingId }),
  mergeSpeakers: (intoId: string, fromId: string) =>
    invoke<MeetingSpeaker[]>('merge_meeting_speakers', { intoId, fromId }),
  listSpeakers: (meetingId: string) => invoke<MeetingSpeaker[]>('list_meeting_speakers', { meetingId }),
  updateSpeaker: (speakerId: string, label: string | null, memberId: string | null) =>
    invoke<MeetingSpeaker[]>('update_meeting_speaker', { speakerId, label, memberId }),
};

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
}

export interface DiarizeOutcome {
  meetingId: string;
  speakers: MeetingSpeaker[];
  assignedSegments: number;
  totalSegments: number;
}

export const DIARIZATION_EVENT = 'diarization-complete';

export const diarizationService = {
  diarize: (meetingId: string) => invoke<DiarizeOutcome>('diarize_meeting', { meetingId }),
  listSpeakers: (meetingId: string) => invoke<MeetingSpeaker[]>('list_meeting_speakers', { meetingId }),
  updateSpeaker: (speakerId: string, label: string | null, memberId: string | null) =>
    invoke<MeetingSpeaker[]>('update_meeting_speaker', { speakerId, label, memberId }),
};

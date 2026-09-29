/**
 * The app's user: profile (name, learned voice) and the "My time" report of
 * tickets they worked on, built from their own lines in meetings.
 */

import { invoke } from '@tauri-apps/api/core';
import type { MeetingSpeaker } from '@/services/diarizationService';

export interface UserProfile {
  displayName: string | null;
  /** Meetings whose voice taught the user's voiceprint. */
  voiceprintSamples: number;
}

export interface MyTimeQuote {
  meetingId: string;
  meetingTitle: string;
  date: string;
  startTime: number | null;
  text: string;
}

export interface MyTicket {
  key: string;
  projectName: string | null;
  description: string | null;
  estimatedHours: number | null;
  estimateBasis: string | null;
  talkSeconds: number;
  mentions: number;
  meetingCount: number;
  firstDate: string;
  lastDate: string;
  quotes: MyTimeQuote[];
}

export interface MyTimeReport {
  dateFrom: string;
  dateTo: string;
  meetings: number;
  meetingsWithMe: number;
  micLines: number;
  voiceLines: number;
  tickets: MyTicket[];
  totalEstimatedHours: number | null;
  totalTalkSeconds: number;
  llmError: string | null;
}

export interface MyTimeRequest {
  projectId: string | null;
  allProjects: boolean;
  /** Inclusive, YYYY-MM-DD */
  dateFrom: string;
  dateTo: string;
}

export const meService = {
  getProfile: () => invoke<UserProfile>('get_user_profile'),
  setDisplayName: (name: string | null) => invoke<UserProfile>('set_user_display_name', { name }),
  setSpeakerIsMe: (speakerId: string, isMe: boolean) =>
    invoke<MeetingSpeaker[]>('set_meeting_speaker_is_me', { speakerId, isMe }),
  myTime: (request: MyTimeRequest) => invoke<MyTimeReport>('my_time_report', { request }),
};

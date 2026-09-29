/**
 * RAG Service
 *
 * Wraps the RAG Tauri commands: index configuration, per-project index status,
 * (re)indexing, and hybrid (semantic + keyword) search over meetings.
 */

import { invoke } from '@tauri-apps/api/core';

export interface RagConfig {
  enabled: boolean;
  embeddingProvider: string;
  embeddingModel: string;
  ollamaEndpoint: string | null;
  extractFacts: boolean;
}

export interface ProjectIndexStatus {
  meetingCount: number;
  indexedMeetings: number;
  partialMeetings: number;
  failedMeetings: number;
  chunkCount: number;
  embeddedChunks: number;
  staleEmbeddings: number;
  lastError: string | null;
}

export interface IndexOutcome {
  meetingId: string;
  status: 'indexed' | 'partial' | 'error';
  chunkCount: number;
  embeddedCount: number;
  error: string | null;
  factCount: number | null;
  factsError: string | null;
}

export interface SearchResult {
  chunkId: string;
  meetingId: string;
  meetingTitle: string;
  meetingDate: string;
  kind: 'transcript' | 'summary' | 'notes';
  text: string;
  startTime: number | null;
  endTime: number | null;
  score: number;
  sources: Array<'vector' | 'lexical'>;
}

export interface SearchResponse {
  results: SearchResult[];
  vectorError: string | null;
}

export interface SearchRequest {
  projectId: string;
  query: string;
  meetingId?: string | null;
  dateFrom?: string | null;
  dateTo?: string | null;
  limit?: number;
}

export interface Citation {
  index: number;
  chunkId: string;
  meetingId: string;
  meetingTitle: string;
  meetingDate: string;
  kind: 'transcript' | 'summary' | 'notes' | 'fact';
  startTime: number | null;
  endTime: number | null;
  excerpt: string;
}

export interface QueryPlan {
  searchQuery: string;
  dateFrom: string | null;
  dateTo: string | null;
  meetingId: string | null;
  tickets: string[];
  factTypes: string[];
}

export type FactType = 'status' | 'blocker' | 'decision' | 'action';

export interface Fact {
  id: string;
  ticket: string | null;
  factType: FactType;
  content: string;
  owner: string | null;
  meetingId: string;
  meetingTitle: string;
  meetingDate: string;
  startTime: number | null;
  chunkId: string | null;
}

export interface TicketSummary {
  entityId: string;
  key: string;
  factCount: number;
  lastMeetingDate: string;
  latestFactType: FactType;
  latestContent: string;
}

export interface Answer {
  answer: string;
  found: boolean;
  citations: Citation[];
  plan: QueryPlan;
  filtersRelaxed: boolean;
  notice: string | null;
}

export interface ConversationTurn {
  question: string;
  answer: string;
}

export const RAG_INDEX_EVENT = 'rag-index-progress';

export const ragService = {
  getConfig: () => invoke<RagConfig>('rag_get_config'),
  saveConfig: (config: RagConfig) => invoke<RagConfig>('rag_save_config', { config }),
  indexStatus: (projectId: string) => invoke<ProjectIndexStatus>('rag_index_status', { projectId }),
  indexMeeting: (meetingId: string) => invoke<IndexOutcome | null>('rag_index_meeting', { meetingId }),
  reindexProject: (projectId: string) => invoke<number>('rag_reindex_project', { projectId }),
  search: (request: SearchRequest) => invoke<SearchResponse>('rag_search', { request }),
  listTickets: (projectId: string) => invoke<TicketSummary[]>('rag_list_tickets', { projectId }),
  ticketFacts: (entityId: string) => invoke<Fact[]>('rag_ticket_facts', { entityId }),
  listFacts: (projectId: string, factType: FactType, limit?: number) =>
    invoke<Fact[]>('rag_list_facts', { projectId, factType, limit: limit ?? null }),
  ask: (projectId: string, question: string, history: ConversationTurn[]) =>
    invoke<Answer>('rag_ask', { request: { projectId, question, history } }),
};

/** Formats seconds as m:ss / h:mm:ss for citations. */
export function formatTimestamp(seconds: number | null): string | null {
  if (seconds === null || seconds === undefined || Number.isNaN(seconds)) return null;
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = s.toString().padStart(2, '0');
  return h > 0 ? `${h}:${m.toString().padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

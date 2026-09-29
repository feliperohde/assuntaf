'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { listen } from '@tauri-apps/api/event';
import { RefreshCw, Search, AlertTriangle } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import {
  ProjectIndexStatus,
  RAG_INDEX_EVENT,
  SearchResponse,
  formatTimestamp,
  ragService,
} from '@/services/ragService';

const KIND_LABELS: Record<string, string> = {
  transcript: 'Transcript',
  summary: 'Summary',
  notes: 'Notes',
};

/** Index status, reindexing and hybrid search for one project's meetings. */
export function ProjectKnowledgePanel({ projectId }: { projectId: string }) {
  const router = useRouter();
  const { setCurrentMeeting } = useSidebar();
  const [status, setStatus] = useState<ProjectIndexStatus | null>(null);
  const [query, setQuery] = useState('');
  const [response, setResponse] = useState<SearchResponse | null>(null);
  const [searching, setSearching] = useState(false);
  const [reindexing, setReindexing] = useState(false);

  const loadStatus = useCallback(async () => {
    try {
      setStatus(await ragService.indexStatus(projectId));
    } catch (error) {
      console.error('Failed to load index status:', error);
    }
  }, [projectId]);

  useEffect(() => {
    setResponse(null);
    loadStatus();
  }, [loadStatus]);

  // Refresh status as background indexing progresses
  useEffect(() => {
    const unlisten = listen(RAG_INDEX_EVENT, () => {
      loadStatus();
    });
    return () => {
      unlisten.then(fn => fn());
    };
  }, [loadStatus]);

  useEffect(() => {
    if (!status || !reindexing) return;
    const done = status.indexedMeetings + status.partialMeetings + status.failedMeetings;
    if (done >= status.meetingCount) setReindexing(false);
  }, [status, reindexing]);

  const handleReindex = async () => {
    try {
      const queued = await ragService.reindexProject(projectId);
      setReindexing(queued > 0);
      toast.info(queued > 0 ? `Reindexing ${queued} meeting(s)…` : 'This project has no meetings yet');
    } catch (error) {
      toast.error('Failed to start reindexing', { description: String(error) });
    }
  };

  const handleSearch = async () => {
    if (!query.trim()) return;
    setSearching(true);
    try {
      setResponse(await ragService.search({ projectId, query: query.trim(), limit: 10 }));
    } catch (error) {
      toast.error('Search failed', { description: String(error) });
    } finally {
      setSearching(false);
    }
  };

  const openMeeting = (meetingId: string, title: string) => {
    setCurrentMeeting({ id: meetingId, title });
    router.push(`/meeting-details?id=${meetingId}`);
  };

  const needsAttention =
    status && (status.partialMeetings > 0 || status.failedMeetings > 0 || status.staleEmbeddings > 0);

  return (
    <div className="border-t border-gray-100 pt-5 space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="font-semibold">Knowledge index</h3>
        <Button variant="outline" size="sm" onClick={handleReindex} disabled={reindexing}>
          <RefreshCw className={`w-4 h-4 mr-2 ${reindexing ? 'animate-spin' : ''}`} />
          {reindexing ? 'Reindexing…' : 'Reindex project'}
        </Button>
      </div>

      {status && (
        <div className="text-sm text-gray-600 space-y-1">
          <p>
            {status.indexedMeetings + status.partialMeetings} of {status.meetingCount} meetings indexed ·{' '}
            {status.chunkCount} passages · {status.embeddedChunks} with semantic vectors
          </p>
          {needsAttention && (
            <p className="flex items-start gap-2 text-amber-700">
              <AlertTriangle className="w-4 h-4 mt-0.5 flex-shrink-0" />
              <span>
                {status.staleEmbeddings > 0 && 'Some passages were indexed with a different model. '}
                {(status.partialMeetings > 0 || status.failedMeetings > 0) && 'Some meetings are only keyword-searchable. '}
                {status.lastError && <span className="block text-xs">Last error: {status.lastError}</span>}
                Reindex after fixing the issue (e.g. start Ollama and pull the embedding model).
              </span>
            </p>
          )}
        </div>
      )}

      <div className="flex gap-2">
        <Input
          value={query}
          onChange={e => setQuery(e.target.value)}
          onKeyDown={e => e.key === 'Enter' && handleSearch()}
          placeholder="Search this project's meetings, e.g. why is ABC-123 blocked?"
        />
        <Button variant="blue" onClick={handleSearch} disabled={searching || !query.trim()}>
          <Search className="w-4 h-4" />
        </Button>
      </div>

      {response?.vectorError && (
        <p className="text-xs text-amber-700">
          Semantic search unavailable ({response.vectorError}). Showing keyword matches only.
        </p>
      )}

      {response && (
        <ul className="space-y-2">
          {response.results.length === 0 && <li className="text-sm text-gray-400">No matches</li>}
          {response.results.map(result => {
            const start = formatTimestamp(result.startTime);
            return (
              <li key={result.chunkId}>
                <button
                  onClick={() => openMeeting(result.meetingId, result.meetingTitle)}
                  className="w-full text-left p-3 rounded-md border border-gray-200 hover:bg-gray-50"
                >
                  <div className="flex items-center gap-2 text-xs text-gray-500 mb-1">
                    <span className="font-medium text-gray-800">{result.meetingTitle}</span>
                    <span>· {result.meetingDate.slice(0, 10)}</span>
                    {start && <span>· {start}</span>}
                    <span className="ml-auto px-1.5 py-0.5 rounded bg-gray-100">{KIND_LABELS[result.kind] ?? result.kind}</span>
                  </div>
                  <p className="text-sm text-gray-700 line-clamp-3 whitespace-pre-line">{result.text}</p>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

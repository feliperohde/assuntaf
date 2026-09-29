'use client';

import React, { Suspense, useCallback, useEffect, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { listen } from '@tauri-apps/api/event';
import { BookOpen, CheckCircle2, ChevronDown, ChevronLeft, ChevronRight, Loader2, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { useProject } from '@/contexts/ProjectContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { FactBadge } from '@/components/ProjectFactsPanel';
import type { KnowledgeView } from '@/components/Knowledge/SidebarKnowledge';
import {
  ASK_HISTORY_EVENT,
  AskHistoryEntry,
  Fact,
  RAG_INDEX_EVENT,
  TicketSummary,
  formatTimestamp,
  ragService,
} from '@/services/ragService';

const PAGE_SIZE = 20;

const VIEWS: { value: KnowledgeView; label: string }[] = [
  { value: 'asks', label: 'Ask history' },
  { value: 'tickets', label: 'Tickets' },
  { value: 'decision', label: 'Decisions' },
  { value: 'action', label: 'Action items' },
];

type Rows =
  | { view: 'asks'; items: AskHistoryEntry[] }
  | { view: 'tickets'; items: TicketSummary[] }
  | { view: 'decision' | 'action'; items: Fact[] };

function isView(value: string | null): value is KnowledgeView {
  return VIEWS.some(v => v.value === value);
}

function KnowledgeContent() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const { activeProject, activeProjectId } = useProject();
  const { setCurrentMeeting } = useSidebar();

  const paramView = searchParams.get('view');
  const view: KnowledgeView = isView(paramView) ? paramView : 'asks';
  const focusTicket = searchParams.get('ticket');

  const [pageIndex, setPageIndex] = useState(0);
  const [rows, setRows] = useState<Rows | null>(null);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [timeline, setTimeline] = useState<Fact[]>([]);

  useEffect(() => setPageIndex(0), [view, activeProjectId]);

  const load = useCallback(async () => {
    if (!activeProjectId) return;
    setLoading(true);
    const offset = pageIndex * PAGE_SIZE;
    try {
      if (view === 'asks') {
        const page = await ragService.askHistory(activeProjectId, PAGE_SIZE, offset);
        setRows({ view, items: page.items });
        setTotal(page.total);
      } else if (view === 'tickets') {
        const page = await ragService.ticketsPage(activeProjectId, PAGE_SIZE, offset);
        setRows({ view, items: page.items });
        setTotal(page.total);
      } else {
        const page = await ragService.factsPage(activeProjectId, view, PAGE_SIZE, offset);
        setRows({ view, items: page.items });
        setTotal(page.total);
      }
    } catch (error) {
      console.error('Failed to load list:', error);
      setRows(null);
      setTotal(0);
    } finally {
      setLoading(false);
    }
  }, [activeProjectId, view, pageIndex]);

  useEffect(() => {
    load();
  }, [load]);

  // Deleting the last entry of the last page leaves it empty: step back
  useEffect(() => {
    if (pageIndex > 0 && pageIndex * PAGE_SIZE >= total) {
      setPageIndex(Math.max(0, Math.ceil(total / PAGE_SIZE) - 1));
    }
  }, [pageIndex, total]);

  useEffect(() => {
    const unlisten = listen(RAG_INDEX_EVENT, () => load());
    window.addEventListener(ASK_HISTORY_EVENT, load);
    return () => {
      unlisten.then(fn => fn());
      window.removeEventListener(ASK_HISTORY_EVENT, load);
    };
  }, [load]);

  const toggleTicket = useCallback(async (entityId: string) => {
    setExpanded(current => (current === entityId ? null : entityId));
    try {
      setTimeline(await ragService.ticketFacts(entityId));
    } catch (error) {
      console.error('Failed to load ticket timeline:', error);
      setTimeline([]);
    }
  }, []);

  // A ticket picked in the sidebar opens with its timeline expanded
  useEffect(() => {
    if (view === 'tickets' && focusTicket) {
      setExpanded(focusTicket);
      ragService.ticketFacts(focusTicket).then(setTimeline).catch(() => setTimeline([]));
    }
  }, [view, focusTicket]);

  const setView = (next: KnowledgeView) => router.push(`/knowledge?view=${next}`);

  const openMeeting = (fact: Fact) => {
    setCurrentMeeting({ id: fact.meetingId, title: fact.meetingTitle });
    router.push(`/meeting-details?id=${fact.meetingId}`);
  };

  const deleteEntry = async (id: string) => {
    try {
      await ragService.deleteAskHistory(id);
      window.dispatchEvent(new Event(ASK_HISTORY_EVENT));
    } catch (error) {
      console.error('Failed to delete saved answer:', error);
    }
  };

  const factLine = (fact: Fact, showType: boolean) => {
    const time = formatTimestamp(fact.startTime);
    return (
      <button
        key={fact.id}
        onClick={() => openMeeting(fact)}
        className="w-full text-left p-3 rounded-md bg-white hover:bg-gray-50 border border-gray-200"
      >
        <div className="flex flex-wrap items-center gap-2 text-xs text-gray-500 mb-1">
          {showType && <FactBadge type={fact.factType} />}
          {fact.ticket && <span className="font-mono text-gray-700">{fact.ticket}</span>}
          <span className="font-medium text-gray-800">{fact.meetingTitle}</span>
          <span>· {fact.meetingDate.slice(0, 10)}</span>
          {time && <span>· {time}</span>}
          {fact.owner && <span className="ml-auto">Owner: {fact.owner}</span>}
        </div>
        <p className="text-sm text-gray-700">{fact.content}</p>
      </button>
    );
  };

  const renderRows = () => {
    if (!rows || rows.view !== view) return null;
    if (rows.items.length === 0) {
      return (
        <p className="text-sm text-gray-400 py-8 text-center">
          {view === 'asks'
            ? 'No questions asked in this project yet.'
            : 'Nothing extracted yet. Facts come from indexed meetings (Settings → Knowledge).'}
        </p>
      );
    }
    switch (rows.view) {
      case 'asks':
        return rows.items.map(entry => (
          <div key={entry.id} className="flex items-start gap-2 p-3 rounded-md bg-white border border-gray-200 hover:bg-gray-50">
            <button className="flex-1 min-w-0 text-left" onClick={() => router.push(`/ask?history=${entry.id}`)}>
              <div className="flex items-center gap-2">
                {entry.found && <CheckCircle2 className="w-4 h-4 flex-shrink-0 text-green-600" />}
                <span className="font-medium text-gray-900">{entry.question}</span>
              </div>
              <p className="text-sm text-gray-600 mt-1 line-clamp-2">{entry.answerPreview || 'No answer found.'}</p>
              <p className="text-xs text-gray-400 mt-1">
                {new Date(entry.createdAt).toLocaleString()} · {entry.citationCount} source(s)
              </p>
            </button>
            <Button variant="ghost" size="sm" onClick={() => deleteEntry(entry.id)} aria-label="Delete from history">
              <Trash2 className="w-4 h-4 text-gray-400" />
            </Button>
          </div>
        ));
      case 'tickets':
        return rows.items.map(ticket => (
          <div key={ticket.entityId} className="rounded-md border border-gray-200 bg-white">
            <button onClick={() => toggleTicket(ticket.entityId)} className="w-full flex items-start gap-2 p-3 text-left hover:bg-gray-50">
              {expanded === ticket.entityId ? (
                <ChevronDown className="w-4 h-4 mt-0.5 text-gray-500" />
              ) : (
                <ChevronRight className="w-4 h-4 mt-0.5 text-gray-500" />
              )}
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="font-mono font-semibold">{ticket.key}</span>
                  <FactBadge type={ticket.latestFactType} />
                  <span className="ml-auto text-xs text-gray-500">
                    {ticket.lastMeetingDate.slice(0, 10)} · {ticket.factCount} mention(s)
                  </span>
                </div>
                <p className="text-sm text-gray-600 mt-1">{ticket.latestContent}</p>
              </div>
            </button>
            {expanded === ticket.entityId && (
              <div className="px-3 pb-3 space-y-2">{timeline.map(fact => factLine(fact, true))}</div>
            )}
          </div>
        ));
      default:
        return rows.items.map(fact => factLine(fact, false));
    }
  };

  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const first = total === 0 ? 0 : pageIndex * PAGE_SIZE + 1;
  const last = Math.min(total, (pageIndex + 1) * PAGE_SIZE);

  return (
    <div className="h-screen bg-gray-50 flex flex-col">
      <div className="border-b border-gray-200">
        <div className="max-w-4xl mx-auto px-8 py-6">
          <h1 className="text-3xl font-bold flex items-center gap-3">
            <BookOpen className="w-7 h-7 text-gray-600" /> Knowledge
          </h1>
          <p className="text-sm text-gray-500 mt-1">
            From the meetings of <span className="font-medium">{activeProject?.name ?? 'this project'}</span>.
          </p>
          <div className="flex items-center gap-1 mt-4">
            {VIEWS.map(v => (
              <button
                key={v.value}
                onClick={() => setView(v.value)}
                className={`px-3 py-1.5 text-sm rounded-md ${view === v.value ? 'bg-gray-900 text-white' : 'text-gray-600 hover:bg-gray-100'}`}
              >
                {v.label}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        <div className="max-w-4xl mx-auto px-8 py-6 space-y-2">
          {loading && !rows && (
            <div className="flex items-center gap-2 text-sm text-gray-500">
              <Loader2 className="w-4 h-4 animate-spin" /> Loading…
            </div>
          )}
          {renderRows()}
        </div>
      </div>

      {total > PAGE_SIZE && (
        <div className="border-t border-gray-200 bg-white">
          <div className="max-w-4xl mx-auto px-8 py-3 flex items-center justify-between text-sm text-gray-600">
            <span>
              {first}–{last} of {total}
            </span>
            <div className="flex items-center gap-2">
              <Button variant="outline" size="sm" disabled={pageIndex === 0 || loading} onClick={() => setPageIndex(p => p - 1)}>
                <ChevronLeft className="w-4 h-4" /> Previous
              </Button>
              <span>
                Page {pageIndex + 1} / {pageCount}
              </span>
              <Button
                variant="outline"
                size="sm"
                disabled={pageIndex + 1 >= pageCount || loading}
                onClick={() => setPageIndex(p => p + 1)}
              >
                Next <ChevronRight className="w-4 h-4" />
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default function KnowledgePage() {
  return (
    <Suspense fallback={<div className="h-screen bg-gray-50" />}>
      <KnowledgeContent />
    </Suspense>
  );
}

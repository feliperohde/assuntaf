'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { listen } from '@tauri-apps/api/event';
import { ChevronDown, ChevronRight, CheckCircle2, CircleHelp, History, ListTodo, Ticket, Gavel } from 'lucide-react';
import { useProject } from '@/contexts/ProjectContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { FactBadge } from '@/components/ProjectFactsPanel';
import {
  ASK_HISTORY_EVENT,
  AskHistoryEntry,
  Fact,
  Page,
  RAG_INDEX_EVENT,
  TicketSummary,
  ragService,
} from '@/services/ragService';

export type KnowledgeView = 'asks' | 'tickets' | 'decision' | 'action';

/** Items shown per accordion; "See all" opens the paginated list. */
const PREVIEW_SIZE = 10;
const OPEN_KEY = 'assunta.sidebar.knowledge.open';

const SECTIONS: { view: KnowledgeView; label: string; icon: React.ElementType }[] = [
  { view: 'asks', label: 'Ask history', icon: History },
  { view: 'tickets', label: 'Tickets', icon: Ticket },
  { view: 'decision', label: 'Decisions', icon: Gavel },
  { view: 'action', label: 'Action items', icon: ListTodo },
];

type SectionData =
  | { view: 'asks'; page: Page<AskHistoryEntry> }
  | { view: 'tickets'; page: Page<TicketSummary> }
  | { view: 'decision' | 'action'; page: Page<Fact> };

function loadSection(projectId: string, view: KnowledgeView): Promise<SectionData> {
  switch (view) {
    case 'asks':
      return ragService.askHistory(projectId, PREVIEW_SIZE).then(page => ({ view, page }));
    case 'tickets':
      return ragService.ticketsPage(projectId, PREVIEW_SIZE).then(page => ({ view, page }));
    default:
      return ragService.factsPage(projectId, view, PREVIEW_SIZE).then(page => ({ view, page }));
  }
}

function readOpen(): KnowledgeView[] {
  try {
    const raw = localStorage.getItem(OPEN_KEY);
    return raw ? (JSON.parse(raw) as KnowledgeView[]) : [];
  } catch {
    return [];
  }
}

/**
 * Accordions with the latest questions, tickets, decisions and action items of
 * the active project. Sections load when opened and refresh after indexing or a
 * new question.
 */
export function SidebarKnowledge() {
  const router = useRouter();
  const { activeProjectId } = useProject();
  const { setCurrentMeeting } = useSidebar();
  const [open, setOpen] = useState<KnowledgeView[]>([]);
  const [data, setData] = useState<Partial<Record<KnowledgeView, SectionData>>>({});

  useEffect(() => setOpen(readOpen()), []);

  const refresh = useCallback(
    async (views: KnowledgeView[]) => {
      if (!activeProjectId) return;
      const loaded = await Promise.all(
        views.map(view =>
          loadSection(activeProjectId, view).catch(error => {
            console.error(`Failed to load ${view}:`, error);
            return null;
          })
        )
      );
      setData(prev => {
        const next = { ...prev };
        loaded.forEach(section => {
          if (section) next[section.view] = section;
        });
        return next;
      });
    },
    [activeProjectId]
  );

  // Project switch invalidates everything
  useEffect(() => {
    setData({});
  }, [activeProjectId]);

  useEffect(() => {
    refresh(open);
  }, [open, refresh]);

  useEffect(() => {
    const unlisten = listen(RAG_INDEX_EVENT, () => refresh(open.filter(v => v !== 'asks')));
    const onAsk = () => {
      if (open.includes('asks')) refresh(['asks']);
    };
    window.addEventListener(ASK_HISTORY_EVENT, onAsk);
    return () => {
      unlisten.then(fn => fn());
      window.removeEventListener(ASK_HISTORY_EVENT, onAsk);
    };
  }, [open, refresh]);

  const toggle = (view: KnowledgeView) => {
    setOpen(prev => {
      const next = prev.includes(view) ? prev.filter(v => v !== view) : [...prev, view];
      try {
        localStorage.setItem(OPEN_KEY, JSON.stringify(next));
      } catch {
        // Only a convenience
      }
      return next;
    });
  };

  const openFact = (fact: Fact) => {
    setCurrentMeeting({ id: fact.meetingId, title: fact.meetingTitle });
    router.push(`/meeting-details?id=${fact.meetingId}`);
  };

  const itemClass = 'w-full text-left px-2 py-1.5 rounded-md hover:bg-gray-100 text-sm';

  const renderItems = (section: SectionData) => {
    switch (section.view) {
      case 'asks':
        return section.page.items.map(entry => (
          <button key={entry.id} className={itemClass} onClick={() => router.push(`/ask?history=${entry.id}`)} title={entry.answerPreview}>
            <span className="flex items-center gap-1.5">
              {entry.found ? (
                <CheckCircle2 className="w-3 h-3 flex-shrink-0 text-green-600" aria-label="Answered" />
              ) : (
                <CircleHelp className="w-3 h-3 flex-shrink-0 text-gray-400" aria-label="Not found in meetings" />
              )}
              <span className="truncate text-gray-800">{entry.question}</span>
            </span>
            <span className="block text-xs text-gray-400">{entry.createdAt.slice(0, 10)}</span>
          </button>
        ));
      case 'tickets':
        return section.page.items.map(ticket => (
          <button
            key={ticket.entityId}
            className={itemClass}
            onClick={() => router.push(`/knowledge?view=tickets&ticket=${ticket.entityId}`)}
            title={ticket.latestContent}
          >
            <span className="flex items-center gap-1.5">
              <span className="font-mono font-semibold text-gray-800">{ticket.key}</span>
              <FactBadge type={ticket.latestFactType} />
            </span>
            <span className="block truncate text-xs text-gray-500">{ticket.latestContent}</span>
          </button>
        ));
      default:
        return section.page.items.map(fact => (
          <button key={fact.id} className={itemClass} onClick={() => openFact(fact)} title={fact.content}>
            <span className="block line-clamp-2 text-gray-800">{fact.content}</span>
            <span className="block truncate text-xs text-gray-400">
              {fact.meetingDate.slice(0, 10)}
              {fact.owner && ` · ${fact.owner}`}
            </span>
          </button>
        ));
    }
  };

  return (
    <div className="mx-3 mt-2 space-y-0.5">
      {SECTIONS.map(({ view, label, icon: Icon }) => {
        const isOpen = open.includes(view);
        const section = data[view];
        const total = section?.page.total;
        return (
          <div key={view}>
            <button
              onClick={() => toggle(view)}
              aria-expanded={isOpen}
              className="w-full flex items-center gap-2 px-2 h-8 rounded-lg hover:bg-gray-100 text-sm font-semibold text-gray-700"
            >
              {isOpen ? <ChevronDown className="w-3.5 h-3.5 text-gray-500" /> : <ChevronRight className="w-3.5 h-3.5 text-gray-500" />}
              <Icon className="w-4 h-4 text-gray-600" />
              <span>{label}</span>
              {total !== undefined && <span className="ml-auto text-xs font-normal text-gray-400">{total}</span>}
            </button>
            {isOpen && (
              <div className="pl-5 pb-1">
                {!section && <p className="px-2 py-1 text-xs text-gray-400">Loading…</p>}
                {section && section.page.items.length === 0 && (
                  <p className="px-2 py-1 text-xs text-gray-400">
                    {view === 'asks' ? 'No questions yet.' : 'Nothing extracted from meetings yet.'}
                  </p>
                )}
                {section && renderItems(section)}
                {section && section.page.total > 0 && (
                  <button
                    onClick={() => router.push(`/knowledge?view=${view}`)}
                    className="px-2 py-1 text-xs font-medium text-blue-600 hover:underline"
                  >
                    See all{section.page.total > PREVIEW_SIZE ? ` (${section.page.total})` : ''} →
                  </button>
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

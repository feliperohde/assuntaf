'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { listen } from '@tauri-apps/api/event';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import {
  Fact,
  FactType,
  RAG_INDEX_EVENT,
  TicketSummary,
  formatTimestamp,
  ragService,
} from '@/services/ragService';
import { useI18n, type MessageKey } from '@/i18n';

export const FACT_STYLES: Record<FactType, { label: MessageKey; className: string }> = {
  blocker: { label: 'fact.blocker', className: 'bg-red-100 text-red-700' },
  status: { label: 'fact.status', className: 'bg-blue-100 text-blue-700' },
  decision: { label: 'fact.decision', className: 'bg-green-100 text-green-700' },
  action: { label: 'fact.action', className: 'bg-amber-100 text-amber-800' },
};

type Tab = 'tickets' | 'decision' | 'action';

export function FactBadge({ type }: { type: FactType }) {
  const { t } = useI18n();
  const style = FACT_STYLES[type] ?? FACT_STYLES.status;
  return <span className={`px-1.5 py-0.5 rounded text-xs font-medium ${style.className}`}>{t(style.label)}</span>;
}

/** Tickets, decisions and action items extracted from a project's meetings. */
export function ProjectFactsPanel({ projectId }: { projectId: string }) {
  const { t } = useI18n();
  const router = useRouter();
  const { setCurrentMeeting } = useSidebar();
  const [tab, setTab] = useState<Tab>('tickets');
  const [tickets, setTickets] = useState<TicketSummary[]>([]);
  const [facts, setFacts] = useState<Fact[]>([]);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [timeline, setTimeline] = useState<Fact[]>([]);

  const load = useCallback(async () => {
    try {
      if (tab === 'tickets') {
        setTickets(await ragService.listTickets(projectId));
      } else {
        setFacts(await ragService.listFacts(projectId, tab));
      }
    } catch (error) {
      console.error('Failed to load facts:', error);
    }
  }, [projectId, tab]);

  useEffect(() => {
    setExpanded(null);
    load();
  }, [load]);

  // Extraction runs after indexing; refresh when it finishes
  useEffect(() => {
    const unlisten = listen(RAG_INDEX_EVENT, () => {
      load();
    });
    return () => {
      unlisten.then(fn => fn());
    };
  }, [load]);

  const toggleTicket = async (entityId: string) => {
    if (expanded === entityId) {
      setExpanded(null);
      return;
    }
    setExpanded(entityId);
    try {
      setTimeline(await ragService.ticketFacts(entityId));
    } catch (error) {
      console.error('Failed to load ticket timeline:', error);
      setTimeline([]);
    }
  };

  const openMeeting = (fact: Fact) => {
    setCurrentMeeting({ id: fact.meetingId, title: fact.meetingTitle });
    router.push(`/meeting-details?id=${fact.meetingId}`);
  };

  const factLine = (fact: Fact, showType: boolean) => {
    const time = formatTimestamp(fact.startTime);
    return (
      <button
        key={fact.id}
        onClick={() => openMeeting(fact)}
        className="w-full text-left p-2 rounded-md hover:bg-gray-50 border border-gray-100"
      >
        <div className="flex items-center gap-2 text-xs text-gray-500 mb-1">
          {showType && <FactBadge type={fact.factType} />}
          {fact.ticket && <span className="font-mono text-gray-700">{fact.ticket}</span>}
          <span className="font-medium text-gray-800">{fact.meetingTitle}</span>
          <span>· {fact.meetingDate.slice(0, 10)}</span>
          {time && <span>· {time}</span>}
          {fact.owner && <span className="ml-auto">{t('facts.owner', { name: fact.owner })}</span>}
        </div>
        <p className="text-sm text-gray-700">{fact.content}</p>
      </button>
    );
  };

  const TABS: { value: Tab; label: string }[] = [
    { value: 'tickets', label: t('facts.tickets') },
    { value: 'decision', label: t('facts.decisions') },
    { value: 'action', label: t('facts.actions') },
  ];

  return (
    <div className="border-t border-gray-100 pt-5 space-y-3">
      <div className="flex items-center gap-1">
        {TABS.map(t => (
          <button
            key={t.value}
            onClick={() => setTab(t.value)}
            className={`px-3 py-1.5 text-sm rounded-md ${tab === t.value ? 'bg-gray-900 text-white' : 'text-gray-600 hover:bg-gray-100'}`}
          >
            {t.label}
          </button>
        ))}
      </div>

      {tab === 'tickets' ? (
        <ul className="space-y-1">
          {tickets.length === 0 && (
            <li className="text-sm text-gray-400">
              {t('facts.noTickets')}
            </li>
          )}
          {tickets.map(ticket => (
            <li key={ticket.entityId} className="rounded-md border border-gray-200">
              <button
                onClick={() => toggleTicket(ticket.entityId)}
                className="w-full flex items-start gap-2 p-3 text-left hover:bg-gray-50"
              >
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
                      {t('facts.mentions', { date: ticket.lastMeetingDate.slice(0, 10), count: ticket.factCount })}
                    </span>
                  </div>
                  <p className="text-sm text-gray-600 mt-1 line-clamp-2">{ticket.latestContent}</p>
                </div>
              </button>
              {expanded === ticket.entityId && (
                <div className="px-3 pb-3 space-y-2">{timeline.map(fact => factLine(fact, true))}</div>
              )}
            </li>
          ))}
        </ul>
      ) : (
        <div className="space-y-2">
          {facts.length === 0 && <p className="text-sm text-gray-400">{t('facts.nothing')}</p>}
          {facts.map(fact => factLine(fact, false))}
        </div>
      )}
    </div>
  );
}

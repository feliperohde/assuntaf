'use client';

import React, { Suspense, useEffect, useRef, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { MessageSquareText, Send, Loader2, AlertTriangle, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { useProject } from '@/contexts/ProjectContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { ASK_HISTORY_EVENT, Answer, ConversationTurn, formatTimestamp, ragService } from '@/services/ragService';

interface Message {
  question: string;
  answer?: Answer;
  error?: string;
}

const EXAMPLES = [
  'What was discussed in the last meeting?',
  'Why is ticket ABC-123 blocked?',
  'On which day did we decide on the release date?',
];

const KIND_LABELS: Record<string, string> = { transcript: 'Transcript', summary: 'Summary', notes: 'Notes', fact: 'Recorded fact' };

function AskContent() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const historyId = searchParams.get('history');
  const { activeProject, activeProjectId, setActiveProjectId } = useProject();
  const { setCurrentMeeting } = useSidebar();
  // Conversation per project, kept for this session
  const [conversations, setConversations] = useState<Record<string, Message[]>>({});
  const [question, setQuestion] = useState('');
  const [loading, setLoading] = useState(false);
  const bottomRef = useRef<HTMLDivElement>(null);

  const messages = conversations[activeProjectId] ?? [];

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages.length, loading]);

  // Reopen a saved answer picked from the history (sidebar or Knowledge page)
  useEffect(() => {
    if (!historyId) return;
    let cancelled = false;
    ragService
      .askHistoryItem(historyId)
      .then(item => {
        if (cancelled || !item) return;
        if (item.projectId !== activeProjectId) setActiveProjectId(item.projectId);
        setConversations(prev => {
          const list = prev[item.projectId] ?? [];
          if (list.some(m => m.answer?.historyId === item.id)) return prev;
          return { ...prev, [item.projectId]: [...list, { question: item.question, answer: { ...item.answer, historyId: item.id } }] };
        });
      })
      .catch(error => console.error('Failed to load saved answer:', error));
    return () => {
      cancelled = true;
    };
    // Only when a different entry is requested
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [historyId]);

  const setMessages = (update: (prev: Message[]) => Message[]) =>
    setConversations(prev => ({ ...prev, [activeProjectId]: update(prev[activeProjectId] ?? []) }));

  const ask = async (text: string) => {
    const q = text.trim();
    if (!q || loading) return;
    const projectId = activeProjectId;
    const history: ConversationTurn[] = messages
      .filter(m => m.answer?.found)
      .map(m => ({ question: m.question, answer: m.answer!.answer }));

    setQuestion('');
    setLoading(true);
    setMessages(prev => [...prev, { question: q }]);
    try {
      const answer = await ragService.ask(projectId, q, history);
      window.dispatchEvent(new Event(ASK_HISTORY_EVENT));
      setConversations(prev => {
        const list = [...(prev[projectId] ?? [])];
        list[list.length - 1] = { question: q, answer };
        return { ...prev, [projectId]: list };
      });
    } catch (error) {
      setConversations(prev => {
        const list = [...(prev[projectId] ?? [])];
        list[list.length - 1] = { question: q, error: String(error) };
        return { ...prev, [projectId]: list };
      });
    } finally {
      setLoading(false);
    }
  };

  const openMeeting = (meetingId: string, title: string) => {
    setCurrentMeeting({ id: meetingId, title });
    router.push(`/meeting-details?id=${meetingId}`);
  };

  return (
    <div className="h-screen bg-gray-50 flex flex-col">
      <div className="border-b border-gray-200 bg-gray-50">
        <div className="max-w-4xl mx-auto px-8 py-6 flex items-center justify-between">
          <div>
            <h1 className="text-3xl font-bold flex items-center gap-3">
              <MessageSquareText className="w-7 h-7 text-gray-600" /> Ask
            </h1>
            <p className="text-sm text-gray-500 mt-1">
              Questions about the meetings of <span className="font-medium">{activeProject?.name ?? 'this project'}</span>.
              Switch project in the sidebar.
            </p>
          </div>
          {messages.length > 0 && (
            <Button variant="ghost" size="sm" onClick={() => {
                setMessages(() => []);
                if (historyId) router.replace('/ask');
              }} disabled={loading}>
              <Trash2 className="w-4 h-4 mr-2" /> Clear
            </Button>
          )}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        <div className="max-w-4xl mx-auto px-8 py-6 space-y-6">
          {messages.length === 0 && (
            <div className="text-center text-gray-500 py-12 space-y-4">
              <p>Ask anything about what was said in this project&apos;s meetings.</p>
              <div className="flex flex-wrap justify-center gap-2">
                {EXAMPLES.map(example => (
                  <button
                    key={example}
                    onClick={() => ask(example)}
                    className="px-3 py-1.5 text-sm rounded-full border border-gray-200 bg-white hover:bg-gray-100"
                  >
                    {example}
                  </button>
                ))}
              </div>
            </div>
          )}

          {messages.map((message, i) => (
            <div key={i} className="space-y-3">
              <div className="flex justify-end">
                <div className="max-w-[80%] rounded-lg bg-blue-600 text-white px-4 py-2 whitespace-pre-wrap">
                  {message.question}
                </div>
              </div>

              {!message.answer && !message.error && (
                <div className="flex items-center gap-2 text-sm text-gray-500">
                  <Loader2 className="w-4 h-4 animate-spin" /> Searching meetings…
                </div>
              )}

              {message.error && (
                <div className="flex items-start gap-2 text-sm text-red-700 bg-red-50 border border-red-100 rounded-lg p-3">
                  <AlertTriangle className="w-4 h-4 mt-0.5 flex-shrink-0" /> {message.error}
                </div>
              )}

              {message.answer && (
                <div className="rounded-lg bg-white border border-gray-200 p-4 space-y-3">
                  {message.answer.found ? (
                    <div className="prose prose-sm max-w-none">
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>{message.answer.answer}</ReactMarkdown>
                    </div>
                  ) : (
                    <p className="text-sm text-gray-600">
                      I couldn&apos;t find anything about this in this project&apos;s indexed meetings. If the meetings are
                      recent or older than the index, try &quot;Reindex project&quot; on the Projects page.
                    </p>
                  )}

                  {(message.answer.notice || message.answer.filtersRelaxed) && (
                    <p className="text-xs text-amber-700">
                      {message.answer.filtersRelaxed && 'No meetings matched the dates or meeting in the question, so all meetings were searched. '}
                      {message.answer.notice && `Semantic search unavailable (${message.answer.notice}); keyword matches only.`}
                    </p>
                  )}

                  {message.answer.citations.length > 0 && (
                    <div className="border-t border-gray-100 pt-3 space-y-2">
                      <p className="text-xs font-medium text-gray-500 uppercase tracking-wide">Sources</p>
                      {message.answer.citations.map(citation => {
                        const start = formatTimestamp(citation.startTime);
                        return (
                          <button
                            key={citation.chunkId}
                            onClick={() => openMeeting(citation.meetingId, citation.meetingTitle)}
                            className="w-full text-left p-2 rounded-md hover:bg-gray-50 border border-gray-100"
                          >
                            <div className="flex items-center gap-2 text-xs text-gray-500">
                              <span className="font-semibold text-blue-600">[{citation.index}]</span>
                              <span className="font-medium text-gray-800">{citation.meetingTitle}</span>
                              <span>· {citation.meetingDate.slice(0, 10)}</span>
                              {start && <span>· {start}</span>}
                              <span className="ml-auto px-1.5 py-0.5 rounded bg-gray-100">
                                {KIND_LABELS[citation.kind] ?? citation.kind}
                              </span>
                            </div>
                            <p className="text-xs text-gray-600 mt-1 line-clamp-2">{citation.excerpt}</p>
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>
              )}
            </div>
          ))}
          <div ref={bottomRef} />
        </div>
      </div>

      <div className="border-t border-gray-200 bg-white">
        <div className="max-w-4xl mx-auto px-8 py-4 flex gap-2">
          <Textarea
            rows={2}
            value={question}
            onChange={e => setQuestion(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                ask(question);
              }
            }}
            placeholder="Ask about this project's meetings… (Enter to send, Shift+Enter for a new line)"
            className="resize-none"
          />
          <Button variant="blue" onClick={() => ask(question)} disabled={loading || !question.trim()} className="self-end">
            {loading ? <Loader2 className="w-4 h-4 animate-spin" /> : <Send className="w-4 h-4" />}
          </Button>
        </div>
      </div>
    </div>
  );
}

export default function AskPage() {
  return (
    <Suspense fallback={<div className="h-screen bg-gray-50" />}>
      <AskContent />
    </Suspense>
  );
}

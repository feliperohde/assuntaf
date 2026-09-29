'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { listen } from '@tauri-apps/api/event';
import { ArrowRight, MessageSquareText } from 'lucide-react';
import { useProject } from '@/contexts/ProjectContext';
import { useI18n, type MessageKey } from '@/i18n';
import { RAG_INDEX_EVENT, ragService } from '@/services/ragService';

const EXAMPLES: MessageKey[] = ['ask.example1', 'ask.example2', 'ask.example3'];

/**
 * Ask box for the Home welcome screen, shown once the active project has
 * indexed meetings. Questions open the Ask page, which answers them.
 */
export function HomeAsk() {
  const router = useRouter();
  const { t } = useI18n();
  const { activeProject, activeProjectId } = useProject();
  const [hasIndex, setHasIndex] = useState(false);
  const [question, setQuestion] = useState('');

  const refresh = useCallback(async () => {
    if (!activeProjectId) return;
    try {
      const status = await ragService.indexStatus(activeProjectId);
      setHasIndex(status.chunkCount > 0);
    } catch {
      setHasIndex(false);
    }
  }, [activeProjectId]);

  useEffect(() => {
    refresh();
    const unlisten = listen(RAG_INDEX_EVENT, () => refresh());
    return () => {
      unlisten.then(fn => fn());
    };
  }, [refresh]);

  if (!hasIndex) return null;

  const ask = (text: string) => {
    const q = text.trim();
    if (q) router.push(`/ask?q=${encodeURIComponent(q)}`);
  };

  return (
    <div className="mt-8 text-left">
      <div className="flex items-center gap-2 mb-2 text-sm font-medium text-gray-700">
        <MessageSquareText className="w-4 h-4 text-indigo-500" />
        {t('home.askTitle', { project: activeProject?.name ?? t('ask.thisProject') })}
      </div>
      <form
        onSubmit={e => {
          e.preventDefault();
          ask(question);
        }}
        className="flex items-center gap-2 rounded-xl border border-gray-200 bg-white px-3 py-2 shadow-sm focus-within:ring-2 focus-within:ring-indigo-200"
      >
        <input
          value={question}
          onChange={e => setQuestion(e.target.value)}
          placeholder={t('home.askPlaceholder')}
          className="flex-1 bg-transparent text-sm text-gray-800 placeholder:text-gray-400 focus:outline-none"
          aria-label={t('home.askPlaceholder')}
        />
        <button
          type="submit"
          disabled={!question.trim()}
          aria-label={t('nav.ask')}
          className="w-8 h-8 flex items-center justify-center rounded-lg bg-indigo-600 text-white disabled:bg-gray-200 disabled:text-gray-400 transition-colors"
        >
          <ArrowRight className="w-4 h-4" />
        </button>
      </form>
      <div className="flex flex-wrap gap-2 mt-3">
        {EXAMPLES.map(key => (
          <button
            key={key}
            onClick={() => ask(t(key))}
            className="px-3 py-1 text-xs rounded-full border border-gray-200 text-gray-600 hover:bg-gray-100"
          >
            {t(key)}
          </button>
        ))}
      </div>
    </div>
  );
}

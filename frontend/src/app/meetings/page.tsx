'use client';

import React, { useEffect, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { ChevronLeft, ChevronRight, DatabaseZap, File, Loader2, NotebookPen, Search } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useProject } from '@/contexts/ProjectContext';
import { useReindexMeeting } from '@/hooks/useReindexMeeting';
import { useI18n } from '@/i18n';

const PAGE_SIZE = 20;

/** Every meeting of the active project, newest first, paginated ("See all" in the sidebar). */
export default function MeetingsPage() {
  const { t, locale } = useI18n();
  const router = useRouter();
  const { meetings, setCurrentMeeting } = useSidebar();
  const { activeProject } = useProject();
  const { reindex, reindexingId } = useReindexMeeting();
  const [filter, setFilter] = useState('');
  const [pageIndex, setPageIndex] = useState(0);

  const filtered = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    return needle ? meetings.filter(m => m.title.toLowerCase().includes(needle)) : meetings;
  }, [meetings, filter]);

  const pageCount = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  useEffect(() => setPageIndex(0), [filter, activeProject?.id]);
  useEffect(() => {
    if (pageIndex >= pageCount) setPageIndex(pageCount - 1);
  }, [pageIndex, pageCount]);

  const page = filtered.slice(pageIndex * PAGE_SIZE, (pageIndex + 1) * PAGE_SIZE);
  const first = filtered.length === 0 ? 0 : pageIndex * PAGE_SIZE + 1;
  const last = Math.min(filtered.length, (pageIndex + 1) * PAGE_SIZE);

  const open = (id: string, title: string) => {
    setCurrentMeeting({ id, title });
    router.push(`/meeting-details?id=${id}`);
  };

  const formatDate = (value?: string) => {
    if (!value) return '';
    const date = new Date(value);
    return Number.isNaN(date.getTime())
      ? value
      : date.toLocaleString(locale, { dateStyle: 'medium', timeStyle: 'short' });
  };

  return (
    <div className="h-screen bg-white flex flex-col">
      <div className="border-b border-gray-200">
        <div className="max-w-4xl mx-auto px-8 py-6">
          <h1 className="text-3xl font-bold flex items-center gap-3">
            <NotebookPen className="w-7 h-7 text-gray-600" /> {t('nav.meetingNotes')}
          </h1>
          <p className="text-sm text-gray-500 mt-1">
            {t('knowledge.fromMeetingsOf')}
            <span className="font-medium">{activeProject?.name ?? t('ask.thisProject')}</span>.
          </p>
          <div className="relative mt-4 max-w-sm">
            <Search className="w-4 h-4 text-gray-400 absolute left-3 top-1/2 -translate-y-1/2" />
            <Input
              value={filter}
              onChange={e => setFilter(e.target.value)}
              placeholder={t('meetings.filter')}
              className="pl-9"
            />
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        <div className="max-w-4xl mx-auto px-8 py-6 space-y-2">
          {page.length === 0 && (
            <p className="text-sm text-gray-400 py-8 text-center">
              {filter ? t('knowledge.noMatches') : t('meetings.none')}
            </p>
          )}
          {page.map(meeting => (
            <div
              key={meeting.id}
              className="flex items-center gap-3 p-3 rounded-md border border-gray-200 hover:bg-gray-50"
            >
              <button className="flex-1 min-w-0 flex items-center gap-3 text-left" onClick={() => open(meeting.id, meeting.title)}>
                <span className="flex-shrink-0 flex items-center justify-center w-8 h-8 rounded-full bg-gray-100">
                  <File className="w-4 h-4 text-gray-600" />
                </span>
                <span className="min-w-0">
                  <span className="block font-medium text-gray-900 truncate">{meeting.title}</span>
                  <span className="block text-xs text-gray-500">{formatDate(meeting.createdAt)}</span>
                </span>
              </button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => reindex(meeting.id)}
                disabled={reindexingId === meeting.id}
                title={t('reindex.hint')}
              >
                {reindexingId === meeting.id ? (
                  <Loader2 className="w-4 h-4 mr-2 animate-spin" />
                ) : (
                  <DatabaseZap className="w-4 h-4 mr-2" />
                )}
                {t('reindex.action')}
              </Button>
            </div>
          ))}
        </div>
      </div>

      {filtered.length > PAGE_SIZE && (
        <div className="border-t border-gray-200">
          <div className="max-w-4xl mx-auto px-8 py-3 flex items-center justify-between text-sm text-gray-600">
            <span>{t('knowledge.range', { first, last, total: filtered.length })}</span>
            <div className="flex items-center gap-2">
              <Button variant="outline" size="sm" disabled={pageIndex === 0} onClick={() => setPageIndex(p => p - 1)}>
                <ChevronLeft className="w-4 h-4" /> {t('common.previous')}
              </Button>
              <span>{t('knowledge.page', { page: pageIndex + 1, pages: pageCount })}</span>
              <Button
                variant="outline"
                size="sm"
                disabled={pageIndex + 1 >= pageCount}
                onClick={() => setPageIndex(p => p + 1)}
              >
                {t('common.next')} <ChevronRight className="w-4 h-4" />
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

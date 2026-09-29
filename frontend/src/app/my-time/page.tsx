'use client';

import React, { useEffect, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { AlertTriangle, ChevronDown, ChevronRight, FolderKanban, Globe, Loader2, Mic, Timer } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useProject } from '@/contexts/ProjectContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useI18n, type MessageKey } from '@/i18n';
import { formatTimestamp } from '@/services/ragService';
import { MyTicket, MyTimeReport, UserProfile, meService } from '@/services/meService';

type Preset = 'thisWeek' | 'lastWeek' | 'thisMonth' | 'lastMonth' | 'last30' | 'custom';

const PRESETS: { value: Preset; label: MessageKey }[] = [
  { value: 'thisWeek', label: 'myTime.thisWeek' },
  { value: 'lastWeek', label: 'myTime.lastWeek' },
  { value: 'thisMonth', label: 'myTime.thisMonth' },
  { value: 'lastMonth', label: 'myTime.lastMonth' },
  { value: 'last30', label: 'myTime.last30' },
  { value: 'custom', label: 'myTime.custom' },
];

function iso(date: Date) {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, '0');
  const d = String(date.getDate()).padStart(2, '0');
  return `${y}-${m}-${d}`;
}

/** Inclusive [from, to] for a preset, weeks starting on Monday. */
function presetRange(preset: Preset, today = new Date()): [string, string] {
  const day = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  const monday = new Date(day);
  monday.setDate(day.getDate() - ((day.getDay() + 6) % 7));
  switch (preset) {
    case 'lastWeek': {
      const start = new Date(monday);
      start.setDate(monday.getDate() - 7);
      const end = new Date(monday);
      end.setDate(monday.getDate() - 1);
      return [iso(start), iso(end)];
    }
    case 'thisMonth':
      return [iso(new Date(day.getFullYear(), day.getMonth(), 1)), iso(day)];
    case 'lastMonth':
      return [
        iso(new Date(day.getFullYear(), day.getMonth() - 1, 1)),
        iso(new Date(day.getFullYear(), day.getMonth(), 0)),
      ];
    case 'last30': {
      const start = new Date(day);
      start.setDate(day.getDate() - 29);
      return [iso(start), iso(day)];
    }
    default:
      return [iso(monday), iso(day)];
  }
}

function duration(seconds: number) {
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}h ${m}min`;
  if (m > 0) return `${m}min${s ? ` ${s}s` : ''}`;
  return `${s}s`;
}

/** Hours rounded to the quarter: 6h, 1.5h, 0.25h. */
function hours(value: number) {
  return `${Math.round(value * 4) / 4}h`;
}

export default function MyTimePage() {
  const { t, locale } = useI18n();
  const router = useRouter();
  const { activeProject, activeProjectId } = useProject();
  const { setCurrentMeeting } = useSidebar();

  const [preset, setPreset] = useState<Preset>('thisWeek');
  const [[dateFrom, dateTo], setRange] = useState<[string, string]>(() => presetRange('thisWeek'));
  const [allProjects, setAllProjects] = useState(false);
  const [report, setReport] = useState<MyTimeReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [profile, setProfile] = useState<UserProfile | null>(null);
  const [name, setName] = useState('');

  useEffect(() => {
    meService
      .getProfile()
      .then(p => {
        setProfile(p);
        setName(p.displayName ?? '');
      })
      .catch(() => setProfile(null));
  }, []);

  const choosePreset = (value: Preset) => {
    setPreset(value);
    if (value !== 'custom') setRange(presetRange(value));
  };

  const generate = async () => {
    if (!dateFrom || !dateTo || dateFrom > dateTo) {
      toast.error(t('myTime.invalidPeriod'));
      return;
    }
    setLoading(true);
    setExpanded(null);
    try {
      setReport(await meService.myTime({ projectId: activeProjectId, allProjects, dateFrom, dateTo }));
    } catch (error) {
      toast.error(t('myTime.failed'), { description: String(error) });
    } finally {
      setLoading(false);
    }
  };

  const saveName = async () => {
    try {
      setProfile(await meService.setDisplayName(name.trim() || null));
      toast.success(t('myTime.nameSaved'));
    } catch (error) {
      toast.error(String(error));
    }
  };

  const openQuote = (meetingId: string, title: string) => {
    setCurrentMeeting({ id: meetingId, title });
    router.push(`/meeting-details?id=${meetingId}`);
  };

  const formatDay = (value: string) => {
    const date = new Date(`${value}T12:00:00`);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleDateString(locale, { day: '2-digit', month: 'short' });
  };

  const ticketLine = (ticket: MyTicket) =>
    t('myTime.talkLine', {
      talk: duration(ticket.talkSeconds),
      meetings: ticket.meetingCount,
      from: formatDay(ticket.firstDate),
      to: formatDay(ticket.lastDate),
    });

  const tiles = useMemo(() => {
    if (!report) return [];
    return [
      { label: t('myTime.estimated'), value: report.totalEstimatedHours != null ? hours(report.totalEstimatedHours) : '—' },
      { label: t('myTime.tickets'), value: String(report.tickets.length) },
      { label: t('myTime.meetingsWithMe'), value: `${report.meetingsWithMe}/${report.meetings}` },
      { label: t('myTime.talkTime'), value: duration(report.totalTalkSeconds) },
    ];
  }, [report, t]);

  return (
    <div className="h-screen bg-white flex flex-col">
      <div className="border-b border-gray-200">
        <div className="max-w-4xl mx-auto px-8 py-6 space-y-4">
          <div>
            <h1 className="text-3xl font-bold flex items-center gap-3">
              <Timer className="w-7 h-7 text-gray-600" /> {t('nav.myTime')}
            </h1>
            <p className="text-sm text-gray-500 mt-1">{t('myTime.subtitle')}</p>
          </div>

          <div className="flex flex-wrap items-center gap-1">
            {PRESETS.map(p => (
              <button
                key={p.value}
                onClick={() => choosePreset(p.value)}
                className={`px-3 py-1.5 text-sm rounded-md ${preset === p.value ? 'bg-gray-900 text-white' : 'text-gray-600 hover:bg-gray-100'}`}
              >
                {t(p.label)}
              </button>
            ))}
          </div>

          <div className="flex flex-wrap items-end gap-3">
            <label className="text-xs text-gray-500 space-y-1">
              <span className="block">{t('myTime.from')}</span>
              <Input
                type="date"
                value={dateFrom}
                onChange={e => {
                  setPreset('custom');
                  setRange([e.target.value, dateTo]);
                }}
                className="h-9 w-40"
              />
            </label>
            <label className="text-xs text-gray-500 space-y-1">
              <span className="block">{t('myTime.to')}</span>
              <Input
                type="date"
                value={dateTo}
                onChange={e => {
                  setPreset('custom');
                  setRange([dateFrom, e.target.value]);
                }}
                className="h-9 w-40"
              />
            </label>
            <div className="inline-flex rounded-lg border border-gray-200 p-0.5 text-sm h-9 items-center" role="radiogroup">
              {[false, true].map(all => (
                <button
                  key={String(all)}
                  role="radio"
                  aria-checked={allProjects === all}
                  onClick={() => setAllProjects(all)}
                  className={`flex items-center gap-1.5 px-3 py-1 rounded-md transition-colors ${allProjects === all ? 'bg-gray-900 text-white' : 'text-gray-600 hover:bg-gray-100'}`}
                >
                  {all ? <Globe className="w-3.5 h-3.5" /> : <FolderKanban className="w-3.5 h-3.5" />}
                  {all ? t('ask.scopeAll') : activeProject?.name ?? t('ask.scopeProject')}
                </button>
              ))}
            </div>
            <Button variant="blue" onClick={generate} disabled={loading} className="h-9">
              {loading ? <Loader2 className="w-4 h-4 mr-2 animate-spin" /> : <Timer className="w-4 h-4 mr-2" />}
              {loading ? t('myTime.generating') : t('myTime.generate')}
            </Button>
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        <div className="max-w-4xl mx-auto px-8 py-6 space-y-4">
          {profile && !profile.displayName && (
            <div className="rounded-lg border border-indigo-100 bg-indigo-50/50 p-4 space-y-2">
              <p className="text-sm text-gray-700">{t('myTime.askName')}</p>
              <div className="flex gap-2 max-w-md">
                <Input value={name} onChange={e => setName(e.target.value)} placeholder={t('myTime.namePlaceholder')} />
                <Button variant="outline" onClick={saveName} disabled={!name.trim()}>
                  {t('common.save')}
                </Button>
              </div>
            </div>
          )}

          {!report && !loading && (
            <div className="text-center text-gray-500 py-12 space-y-2">
              <Mic className="w-8 h-8 mx-auto text-gray-300" />
              <p className="text-sm max-w-lg mx-auto">{t('myTime.howItWorks')}</p>
            </div>
          )}

          {report && (
            <>
              <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
                {tiles.map(tile => (
                  <div key={tile.label} className="rounded-lg border border-gray-200 p-3">
                    <p className="text-xs text-gray-500">{tile.label}</p>
                    <p className="text-2xl font-semibold text-gray-900 mt-1">{tile.value}</p>
                  </div>
                ))}
              </div>

              {report.meetings > 0 && report.meetingsWithMe === 0 && (
                <p className="flex items-start gap-2 text-sm text-amber-800 bg-amber-50 border border-amber-100 rounded-md p-3">
                  <AlertTriangle className="w-4 h-4 mt-0.5 flex-shrink-0" /> {t('myTime.notRecognized')}
                </p>
              )}
              {report.llmError && report.tickets.length > 0 && (
                <p className="flex items-start gap-2 text-xs text-amber-700">
                  <AlertTriangle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
                  {t('myTime.noEstimates', { reason: report.llmError })}
                </p>
              )}
              {report.meetings === 0 && <p className="text-sm text-gray-400 text-center py-8">{t('myTime.noMeetings')}</p>}
              {report.meetingsWithMe > 0 && report.tickets.length === 0 && (
                <p className="text-sm text-gray-400 text-center py-8">{t('myTime.noTickets')}</p>
              )}

              <div className="space-y-2">
                {report.tickets.map(ticket => (
                  <div key={ticket.key} className="rounded-lg border border-gray-200">
                    <button
                      onClick={() => setExpanded(expanded === ticket.key ? null : ticket.key)}
                      className="w-full flex items-start gap-3 p-4 text-left hover:bg-gray-50"
                    >
                      {expanded === ticket.key ? (
                        <ChevronDown className="w-4 h-4 mt-1 text-gray-400" />
                      ) : (
                        <ChevronRight className="w-4 h-4 mt-1 text-gray-400" />
                      )}
                      <div className="flex-1 min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="font-mono font-semibold text-gray-900">{ticket.key}</span>
                          {ticket.projectName && allProjects && (
                            <span className="px-1.5 py-0.5 rounded bg-indigo-50 text-indigo-700 text-xs">{ticket.projectName}</span>
                          )}
                        </div>
                        <p className="text-sm text-gray-700 mt-1">{ticket.description ?? ticket.quotes[ticket.quotes.length - 1]?.text}</p>
                        <p className="text-xs text-gray-500 mt-1">{ticketLine(ticket)}</p>
                      </div>
                      <div className="text-right flex-shrink-0">
                        <p className="text-2xl font-semibold text-gray-900">
                          {ticket.estimatedHours != null ? `≈ ${hours(ticket.estimatedHours)}` : '—'}
                        </p>
                        {ticket.estimateBasis && (
                          <p className="text-xs text-gray-500 max-w-[14rem]">{ticket.estimateBasis}</p>
                        )}
                      </div>
                    </button>
                    {expanded === ticket.key && (
                      <div className="px-4 pb-4 pl-11 space-y-2">
                        <p className="text-xs font-medium text-gray-500 uppercase tracking-wide">{t('myTime.whatYouSaid')}</p>
                        {ticket.quotes.map((quote, i) => (
                          <button
                            key={i}
                            onClick={() => openQuote(quote.meetingId, quote.meetingTitle)}
                            className="w-full text-left p-2 rounded-md border border-gray-100 hover:bg-gray-50"
                          >
                            <span className="block text-xs text-gray-500">
                              {formatDay(quote.date)} · {quote.meetingTitle}
                              {quote.startTime != null && ` · ${formatTimestamp(quote.startTime)}`}
                            </span>
                            <span className="block text-sm text-gray-700">“{quote.text}”</span>
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

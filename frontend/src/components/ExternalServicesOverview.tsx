'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Cpu, Globe, Server } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ragService } from '@/services/ragService';
import { useI18n, type MessageKey } from '@/i18n';

type Where = { remote: boolean; detail: string };

interface Row {
  key: string;
  labelKey: MessageKey;
  tab: string;
  where: Where | null;
}

const LOCAL_HOSTS = ['localhost', '127.0.0.1', '::1', '[::1]', '0.0.0.0'];

function hostOf(url: string | null | undefined): string | null {
  if (!url?.trim()) return null;
  try {
    const withScheme = /^[a-z]+:\/\//i.test(url.trim()) ? url.trim() : `http://${url.trim()}`;
    return new URL(withScheme).host;
  } catch {
    return url.trim();
  }
}

function isLocal(url: string | null | undefined): boolean {
  const host = hostOf(url);
  if (!host) return true;
  const name = host.replace(/:\d+$/, '');
  return LOCAL_HOSTS.includes(name);
}

/**
 * Where each heavy part of the app runs (this computer or another server), so a
 * low-RAM machine can offload transcription, the LLM, embeddings and vectors.
 */
export function ExternalServicesOverview({ onConfigure }: { onConfigure: (tab: string) => void }) {
  const { t } = useI18n();
  const [rows, setRows] = useState<Row[]>([]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const [transcript, remoteStt, model, rag] = await Promise.all([
        invoke<any>('api_get_transcript_config').catch(() => null),
        invoke<any>('get_remote_transcription_config').catch(() => null),
        invoke<any>('api_get_model_config').catch(() => null),
        ragService.getConfig().catch(() => null),
      ]);
      let customEndpoint: string | null = null;
      if (model?.provider === 'custom-openai') {
        const custom = await invoke<any>('api_get_custom_openai_config').catch(() => null);
        customEndpoint = custom?.endpoint ?? null;
      }

      const local = (detail: string): Where => ({ remote: false, detail });
      const byUrl = (url: string | null | undefined, fallback: string): Where =>
        isLocal(url) ? local(fallback) : { remote: true, detail: hostOf(url) ?? '' };
      const cloud = (name: string): Where => ({ remote: true, detail: name });

      const sttProvider: string = transcript?.provider ?? 'parakeet';
      const stt: Where =
        sttProvider === 'remote'
          ? byUrl(remoteStt?.endpoint, t('services.thisComputer'))
          : ['localWhisper', 'parakeet'].includes(sttProvider)
            ? local(sttProvider === 'parakeet' ? 'Parakeet' : 'Whisper')
            : cloud(sttProvider);

      const llmProvider: string | undefined = model?.provider;
      const ollamaUrl: string | null = model?.ollamaEndpoint ?? null;
      const llm: Where | null = !llmProvider
        ? null
        : llmProvider === 'ollama'
          ? byUrl(ollamaUrl, `Ollama · ${model?.model ?? ''}`)
          : llmProvider === 'builtin-ai'
            ? local(t('services.builtin'))
            : llmProvider === 'custom-openai'
              ? byUrl(customEndpoint, model?.model ?? 'OpenAI-compatible')
              : cloud(llmProvider);

      const embeddings: Where | null = rag?.enabled
        ? byUrl(rag.ollamaEndpoint ?? ollamaUrl, `Ollama · ${rag.embeddingModel}`)
        : null;
      const vectors: Where | null = rag?.enabled
        ? rag.vectorStore === 'qdrant'
          ? byUrl(rag.qdrantUrl, 'Qdrant')
          : local('SQLite')
        : null;

      if (!cancelled) {
        setRows([
          { key: 'stt', labelKey: 'services.transcription', tab: 'Transcriptionmodels', where: stt },
          { key: 'llm', labelKey: 'services.llm', tab: 'summaryModels', where: llm },
          { key: 'emb', labelKey: 'services.embeddings', tab: 'knowledge', where: embeddings },
          { key: 'vec', labelKey: 'services.vectors', tab: 'knowledge', where: vectors },
        ]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [t]);

  return (
    <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm space-y-4">
      <div>
        <h3 className="text-lg font-semibold text-gray-900 flex items-center gap-2">
          <Server className="h-5 w-5 text-gray-500" /> {t('services.title')}
        </h3>
        <p className="text-sm text-gray-600 mt-1">{t('services.intro')}</p>
      </div>
      <div className="divide-y divide-gray-100 border border-gray-100 rounded-md">
        {rows.map(row => (
          <div key={row.key} className="flex items-center justify-between gap-3 px-3 py-2.5">
            <div className="min-w-0">
              <div className="text-sm font-medium text-gray-900">{t(row.labelKey)}</div>
              <div className="text-xs text-gray-500 flex items-center gap-1 truncate">
                {row.where === null ? (
                  t('services.off')
                ) : row.where.remote ? (
                  <>
                    <Globe className="h-3 w-3 text-blue-600 shrink-0" />
                    <span className="text-blue-700">{t('services.remote')}</span>
                    <span className="truncate">· {row.where.detail}</span>
                  </>
                ) : (
                  <>
                    <Cpu className="h-3 w-3 text-amber-600 shrink-0" />
                    <span className="text-amber-700">{t('services.thisComputer')}</span>
                    {row.where.detail && <span className="truncate">· {row.where.detail}</span>}
                  </>
                )}
              </div>
            </div>
            <Button variant="outline" size="sm" onClick={() => onConfigure(row.tab)}>
              {t('services.configure')}
            </Button>
          </div>
        ))}
      </div>
      <p className="text-xs text-gray-500">{t('services.note')}</p>
    </div>
  );
}

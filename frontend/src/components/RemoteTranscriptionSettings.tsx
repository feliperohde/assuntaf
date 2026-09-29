'use client';

import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AlertTriangle, CheckCircle2, Loader2 } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from './ui/button';
import { Input } from './ui/input';
import { Label } from './ui/label';
import { useI18n } from '@/i18n';

export interface RemoteTranscriptionConfig {
  endpoint: string;
  apiKey: string | null;
  model: string;
}

/** Transcription on an OpenAI-compatible server (faster-whisper-server, LocalAI, Groq, OpenAI…). */
export function RemoteTranscriptionSettings({ onSaved }: { onSaved: (config: RemoteTranscriptionConfig) => void }) {
  const { t } = useI18n();
  const [config, setConfig] = useState<RemoteTranscriptionConfig>({ endpoint: '', apiKey: null, model: '' });
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(null);

  useEffect(() => {
    invoke<RemoteTranscriptionConfig | null>('get_remote_transcription_config')
      .then(saved => saved && setConfig(saved))
      .catch(error => console.error('Failed to load remote transcription config:', error));
  }, []);

  const update = (patch: Partial<RemoteTranscriptionConfig>) => {
    setConfig(prev => ({ ...prev, ...patch }));
    setResult(null);
  };

  const test = async () => {
    setTesting(true);
    try {
      const endpoint = await invoke<string>('test_remote_transcription', { config });
      setResult({ ok: true, message: t('remoteStt.ok', { endpoint }) });
    } catch (error) {
      setResult({ ok: false, message: String(error) });
    } finally {
      setTesting(false);
    }
  };

  const save = async () => {
    setSaving(true);
    try {
      const saved = await invoke<RemoteTranscriptionConfig>('save_remote_transcription_config', { config });
      setConfig(saved);
      toast.success(t('remoteStt.saved'));
      onSaved(saved);
    } catch (error) {
      toast.error(t('remoteStt.saveFailed'), { description: String(error) });
    } finally {
      setSaving(false);
    }
  };

  const ready = config.endpoint.trim() && config.model.trim();

  return (
    <div className="mt-4 space-y-4 rounded-lg border border-gray-200 p-4">
      <p className="text-sm text-gray-600">{t('remoteStt.intro')}</p>
      <div className="space-y-1">
        <Label htmlFor="remote-stt-url">{t('remoteStt.url')}</Label>
        <Input
          id="remote-stt-url"
          value={config.endpoint}
          onChange={e => update({ endpoint: e.target.value })}
          placeholder="http://192.168.3.16:8000/v1"
        />
        <p className="text-xs text-gray-500">{t('remoteStt.urlHelp')}</p>
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div className="space-y-1">
          <Label htmlFor="remote-stt-model">{t('remoteStt.model')}</Label>
          <Input
            id="remote-stt-model"
            value={config.model}
            onChange={e => update({ model: e.target.value })}
            placeholder="Systran/faster-whisper-large-v3"
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor="remote-stt-key">{t('remoteStt.apiKey')}</Label>
          <Input
            id="remote-stt-key"
            type="password"
            value={config.apiKey ?? ''}
            onChange={e => update({ apiKey: e.target.value || null })}
            autoComplete="off"
          />
        </div>
      </div>
      <p className="text-xs text-gray-500">{t('remoteStt.examples')}</p>
      {result && (
        <p
          className={`flex items-start gap-1.5 text-xs rounded-md p-2 border ${result.ok ? 'bg-green-50 border-green-200 text-green-800' : 'bg-amber-50 border-amber-200 text-amber-800'}`}
        >
          {result.ok ? <CheckCircle2 className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" /> : <AlertTriangle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />}
          {result.message}
        </p>
      )}
      <div className="flex gap-2">
        <Button variant="outline" onClick={test} disabled={!ready || testing}>
          {testing ? <Loader2 className="w-4 h-4 animate-spin" /> : t('rag.test')}
        </Button>
        <Button variant="blue" onClick={save} disabled={!ready || saving}>
          {t('remoteStt.use')}
        </Button>
      </div>
    </div>
  );
}

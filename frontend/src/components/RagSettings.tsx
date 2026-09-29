"use client"

import { useEffect, useState } from "react"
import { BrainCircuit, CheckCircle2, AlertTriangle, Loader2 } from "lucide-react"
import { toast } from "sonner"
import { Switch } from "./ui/switch"
import { Input } from "./ui/input"
import { Label } from "./ui/label"
import { Button } from "./ui/button"
import { OllamaProbe, RagConfig, ragService } from "@/services/ragService"
import { useI18n } from "@/i18n"

/** Settings for the meeting knowledge index (embeddings via local Ollama). */
export function RagSettings() {
  const { t } = useI18n();
  const [config, setConfig] = useState<RagConfig | null>(null)
  const [saving, setSaving] = useState(false)
  const [probe, setProbe] = useState<OllamaProbe | null>(null)
  const [testing, setTesting] = useState(false)

  const testConnection = async () => {
    if (!config) return
    setTesting(true)
    try {
      setProbe(await ragService.testOllama(config.ollamaEndpoint, config.embeddingModel))
    } catch (error) {
      toast.error(t('rag.testFailed'), { description: String(error) })
    } finally {
      setTesting(false)
    }
  }

  useEffect(() => {
    ragService
      .getConfig()
      .then(setConfig)
      .catch(error => toast.error(t('rag.loadFailed'), { description: String(error) }))
  }, [])

  if (!config) return null

  const save = async () => {
    setSaving(true)
    try {
      setConfig(await ragService.saveConfig(config))
      toast.success(t('rag.saved'), {
        description: t('rag.savedHelp'),
      })
    } catch (error) {
      toast.error(t('rag.saveFailed'), { description: String(error) })
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="space-y-6">
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm space-y-5">
        <div className="flex items-center justify-between">
          <div>
            <div className="flex items-center gap-2 mb-1">
              <BrainCircuit className="h-5 w-5 text-gray-600" />
              <h3 className="text-lg font-semibold text-gray-900">{t('rag.title')}</h3>
            </div>
            <p className="text-sm text-gray-600">
              {t('rag.intro')}
            </p>
          </div>
          <Switch
            checked={config.enabled}
            onCheckedChange={enabled => setConfig({ ...config, enabled })}
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="rag-model">{t('rag.model')}</Label>
          <Input
            id="rag-model"
            value={config.embeddingModel}
            onChange={e => setConfig({ ...config, embeddingModel: e.target.value })}
            placeholder="bge-m3"
          />
          <p className="text-xs text-gray-500">
            {t('rag.modelHelp', { model: config.embeddingModel || "bge-m3" })}
          </p>
        </div>

        <div className="space-y-2">
          <Label htmlFor="rag-endpoint">{t('rag.server')}</Label>
          <div className="flex gap-2">
            <Input
              id="rag-endpoint"
              value={config.ollamaEndpoint ?? ""}
              onChange={e => {
                setConfig({ ...config, ollamaEndpoint: e.target.value || null })
                setProbe(null)
              }}
              placeholder={t('rag.serverPlaceholder')}
            />
            <Button variant="outline" onClick={testConnection} disabled={testing}>
              {testing ? <Loader2 className="w-4 h-4 animate-spin" /> : t('rag.test')}
            </Button>
          </div>
          <p className="text-xs text-gray-500">
            {t('rag.serverHelp')}
          </p>
          {probe && (
            <div
              className={`text-xs rounded-md p-2 border ${
                probe.reachable && probe.modelAvailable
                  ? "bg-green-50 border-green-200 text-green-800"
                  : "bg-amber-50 border-amber-200 text-amber-800"
              }`}
            >
              {!probe.reachable ? (
                <p className="flex items-start gap-1.5">
                  <AlertTriangle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" /> {probe.error}
                </p>
              ) : probe.modelAvailable ? (
                <p className="flex items-center gap-1.5">
                  <CheckCircle2 className="w-3.5 h-3.5" /> {t('rag.connectedOk', { endpoint: probe.endpoint, model: config.embeddingModel })}
                </p>
              ) : (
                <div className="space-y-1">
                  <p className="flex items-center gap-1.5">
                    <AlertTriangle className="w-3.5 h-3.5" /> {t('rag.connectedMissing', { endpoint: probe.endpoint, model: config.embeddingModel })}
                  </p>
                  {probe.models.length > 0 && (
                    <p>{t('rag.availableModels', { models: probe.models.join(", ") })}</p>
                  )}
                </div>
              )}
            </div>
          )}
        </div>

        <div className="flex items-center justify-between">
          <div>
            <Label>{t('rag.extractFacts')}</Label>
            <p className="text-xs text-gray-500 mt-1">
              {t('rag.extractFactsHelp')}
            </p>
          </div>
          <Switch
            checked={config.extractFacts}
            onCheckedChange={extractFacts => setConfig({ ...config, extractFacts })}
          />
        </div>

        <div className="flex items-center justify-between">
          <div>
            <Label>{t('rag.autoDiarize')}</Label>
            <p className="text-xs text-gray-500 mt-1">
              {t('rag.autoDiarizeHelp')}
            </p>
          </div>
          <Switch
            checked={config.autoDiarize}
            onCheckedChange={autoDiarize => setConfig({ ...config, autoDiarize })}
          />
        </div>

        <Button variant="blue" onClick={save} disabled={saving || !config.embeddingModel.trim()}>
          {t('common.save')}
        </Button>
      </div>
    </div>
  )
}

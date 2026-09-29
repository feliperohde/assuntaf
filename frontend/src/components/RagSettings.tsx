"use client"

import { useEffect, useState } from "react"
import { BrainCircuit, CheckCircle2, AlertTriangle, Loader2 } from "lucide-react"
import { toast } from "sonner"
import { Switch } from "./ui/switch"
import { Input } from "./ui/input"
import { Label } from "./ui/label"
import { Button } from "./ui/button"
import { OllamaProbe, RagConfig, ragService } from "@/services/ragService"

/** Settings for the meeting knowledge index (embeddings via local Ollama). */
export function RagSettings() {
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
      toast.error("Connection test failed", { description: String(error) })
    } finally {
      setTesting(false)
    }
  }

  useEffect(() => {
    ragService
      .getConfig()
      .then(setConfig)
      .catch(error => toast.error("Failed to load knowledge index settings", { description: String(error) }))
  }, [])

  if (!config) return null

  const save = async () => {
    setSaving(true)
    try {
      setConfig(await ragService.saveConfig(config))
      toast.success("Knowledge index settings saved", {
        description: "If you changed the model, reindex your projects from the Projects page.",
      })
    } catch (error) {
      toast.error("Failed to save settings", { description: String(error) })
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
              <h3 className="text-lg font-semibold text-gray-900">Meeting knowledge index</h3>
            </div>
            <p className="text-sm text-gray-600">
              Indexes transcripts, summaries and notes of each project so you can search meetings by meaning,
              not just exact words. Runs locally through Ollama.
            </p>
          </div>
          <Switch
            checked={config.enabled}
            onCheckedChange={enabled => setConfig({ ...config, enabled })}
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="rag-model">Embedding model (Ollama)</Label>
          <Input
            id="rag-model"
            value={config.embeddingModel}
            onChange={e => setConfig({ ...config, embeddingModel: e.target.value })}
            placeholder="bge-m3"
          />
          <p className="text-xs text-gray-500">
            Recommended: <code>bge-m3</code> (multilingual, good for Portuguese). Install with{" "}
            <code>ollama pull {config.embeddingModel || "bge-m3"}</code>.
          </p>
        </div>

        <div className="space-y-2">
          <Label htmlFor="rag-endpoint">Ollama server</Label>
          <div className="flex gap-2">
            <Input
              id="rag-endpoint"
              value={config.ollamaEndpoint ?? ""}
              onChange={e => {
                setConfig({ ...config, ollamaEndpoint: e.target.value || null })
                setProbe(null)
              }}
              placeholder="e.g. 192.168.3.16 — empty uses the summary Ollama or this computer"
            />
            <Button variant="outline" onClick={testConnection} disabled={testing}>
              {testing ? <Loader2 className="w-4 h-4 animate-spin" /> : "Test connection"}
            </Button>
          </div>
          <p className="text-xs text-gray-500">
            Accepts an IP or host name; port 11434 and http:// are added automatically. An Ollama on another machine
            must accept network connections (start it with <code>OLLAMA_HOST=0.0.0.0 ollama serve</code>).
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
                  <CheckCircle2 className="w-3.5 h-3.5" /> Connected to {probe.endpoint}; model{" "}
                  <code>{config.embeddingModel}</code> is available. Save, then reindex your projects.
                </p>
              ) : (
                <div className="space-y-1">
                  <p className="flex items-center gap-1.5">
                    <AlertTriangle className="w-3.5 h-3.5" /> Connected to {probe.endpoint}, but{" "}
                    <code>{config.embeddingModel}</code> is not installed there. Run{" "}
                    <code>ollama pull {config.embeddingModel}</code> on that machine.
                  </p>
                  {probe.models.length > 0 && (
                    <p>Available models: {probe.models.join(", ")}</p>
                  )}
                </div>
              )}
            </div>
          )}
        </div>

        <div className="flex items-center justify-between">
          <div>
            <Label>Extract tickets, decisions and action items</Label>
            <p className="text-xs text-gray-500 mt-1">
              After indexing, the summary model lists ticket status, blockers, decisions and action items of each
              meeting. Enables the Tickets view and more precise answers. Uses the summary model once per meeting.
            </p>
          </div>
          <Switch
            checked={config.extractFacts}
            onCheckedChange={extractFacts => setConfig({ ...config, extractFacts })}
          />
        </div>

        <div className="flex items-center justify-between">
          <div>
            <Label>Detect speakers automatically</Label>
            <p className="text-xs text-gray-500 mt-1">
              Identifies who spoke when in each recorded meeting (runs locally; downloads ~35 MB of models on first
              use). Speakers linked to project members are recognized by voice in later meetings.
            </p>
          </div>
          <Switch
            checked={config.autoDiarize}
            onCheckedChange={autoDiarize => setConfig({ ...config, autoDiarize })}
          />
        </div>

        <Button variant="blue" onClick={save} disabled={saving || !config.embeddingModel.trim()}>
          Save
        </Button>
      </div>
    </div>
  )
}

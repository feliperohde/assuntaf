"use client"

import { useEffect, useState } from "react"
import { BrainCircuit } from "lucide-react"
import { toast } from "sonner"
import { Switch } from "./ui/switch"
import { Input } from "./ui/input"
import { Label } from "./ui/label"
import { Button } from "./ui/button"
import { RagConfig, ragService } from "@/services/ragService"

/** Settings for the meeting knowledge index (embeddings via local Ollama). */
export function RagSettings() {
  const [config, setConfig] = useState<RagConfig | null>(null)
  const [saving, setSaving] = useState(false)

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
          <Label htmlFor="rag-endpoint">Ollama endpoint</Label>
          <Input
            id="rag-endpoint"
            value={config.ollamaEndpoint ?? ""}
            onChange={e => setConfig({ ...config, ollamaEndpoint: e.target.value || null })}
            placeholder="Same as summary settings (default http://localhost:11434)"
          />
        </div>

        <Button variant="blue" onClick={save} disabled={saving || !config.embeddingModel.trim()}>
          Save
        </Button>
      </div>
    </div>
  )
}

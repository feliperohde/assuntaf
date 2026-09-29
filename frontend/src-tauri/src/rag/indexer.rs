//! Builds a meeting's chunks (transcript windows, summary, notes), embeds them and
//! writes them to the index. Runs in the background after a meeting is saved or
//! its summary changes; indexing is serialized so jobs never compete for Ollama.

use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::Mutex;

use super::chunker::{chunk_markdown, chunk_transcript, ChunkOptions, Segment};
use super::embeddings::{EmbeddingProvider, OllamaEmbedder};
use super::entities::extract_meeting_facts;
use super::llm::ConfiguredChatModel;
use super::store::{MeetingRef, NewChunk, RagConfig, RagStore};
use crate::database::repositories::setting::SettingsRepository;
use crate::state::AppState;

const MARKDOWN_CHUNK_CHARS: usize = 1500;

/// Serializes indexing jobs.
static INDEX_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexOutcome {
    pub meeting_id: String,
    pub status: String,
    pub chunk_count: usize,
    pub embedded_count: usize,
    pub error: Option<String>,
    /// Facts extracted (tickets, decisions, actions); None when extraction didn't run.
    pub fact_count: Option<usize>,
    pub facts_error: Option<String>,
}

/// Creates the configured embedding provider. The endpoint falls back to the
/// Ollama endpoint configured for summaries, then to localhost.
pub async fn embedder_from_config(pool: &SqlitePool, config: &RagConfig) -> Box<dyn EmbeddingProvider> {
    let endpoint = match config.ollama_endpoint.clone() {
        Some(endpoint) => Some(endpoint),
        None => SettingsRepository::get_model_config(pool)
            .await
            .ok()
            .flatten()
            .and_then(|s| s.ollama_endpoint),
    };
    Box::new(OllamaEmbedder::new(endpoint.as_deref(), &config.embedding_model))
}

struct MeetingSource {
    meeting: MeetingRef,
    project_name: String,
    segments: Vec<Segment>,
    summary_markdown: Option<String>,
    notes_markdown: Option<String>,
}

async fn load_meeting(pool: &SqlitePool, meeting_id: &str) -> Result<MeetingSource> {
    let row: Option<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT m.title, CAST(m.created_at AS TEXT), m.project_id, p.name
         FROM meetings m LEFT JOIN projects p ON p.id = m.project_id WHERE m.id = ?",
    )
    .bind(meeting_id)
    .fetch_optional(pool)
    .await?;
    let (title, meeting_date, project_id, project_name) =
        row.ok_or_else(|| anyhow!("Meeting {} not found", meeting_id))?;

    let segments: Vec<(String, Option<f64>, Option<f64>, Option<String>)> = sqlx::query_as(
        "SELECT transcript, audio_start_time, audio_end_time, speaker FROM transcripts
         WHERE meeting_id = ? ORDER BY COALESCE(audio_start_time, 0), timestamp",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await?;

    let summary: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT result FROM summary_processes WHERE meeting_id = ? AND status = 'completed'",
    )
    .bind(meeting_id)
    .fetch_optional(pool)
    .await?;

    let notes: Option<(Option<String>,)> =
        sqlx::query_as("SELECT notes_markdown FROM meeting_notes WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await?;

    Ok(MeetingSource {
        meeting: MeetingRef {
            meeting_id: meeting_id.to_string(),
            project_id: project_id
                .unwrap_or_else(|| crate::database::repositories::project::DEFAULT_PROJECT_ID.to_string()),
            title,
            meeting_date,
        },
        project_name: project_name.unwrap_or_default(),
        segments: segments
            .into_iter()
            .map(|(text, start_time, end_time, speaker)| Segment { text, start_time, end_time, speaker })
            .collect(),
        summary_markdown: summary.and_then(|(r,)| r).and_then(|r| summary_markdown(&r)),
        notes_markdown: notes.and_then(|(n,)| n).filter(|n| !n.trim().is_empty()),
    })
}

/// Extracts the markdown from a stored summary result (`{"markdown": "..."}`).
fn summary_markdown(result: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(result).ok()?;
    value
        .get("markdown")
        .and_then(|m| m.as_str())
        .map(str::to_string)
        .filter(|m| !m.trim().is_empty())
}

fn build_chunks(source: &MeetingSource) -> Vec<NewChunk> {
    let mut chunks = Vec::new();
    for draft in chunk_transcript(&source.segments, ChunkOptions::default()) {
        chunks.push(NewChunk {
            kind: "transcript".into(),
            chunk_index: chunks.len() as i64,
            text: draft.text,
            speakers: draft.speakers,
            start_time: draft.start_time,
            end_time: draft.end_time,
            embedding: None,
        });
    }
    for (kind, markdown) in [("summary", &source.summary_markdown), ("notes", &source.notes_markdown)] {
        if let Some(markdown) = markdown {
            for text in chunk_markdown(markdown, MARKDOWN_CHUNK_CHARS) {
                chunks.push(NewChunk {
                    kind: kind.into(),
                    chunk_index: chunks.len() as i64,
                    text,
                    speakers: Vec::new(),
                    start_time: None,
                    end_time: None,
                    embedding: None,
                });
            }
        }
    }
    chunks
}

/// Text sent to the embedding model: the chunk prefixed with where it came from,
/// which markedly improves recall for questions like "what was said on Tuesday's daily".
fn embedding_input(source: &MeetingSource, chunk: &NewChunk) -> String {
    let date = source.meeting.meeting_date.get(..10).unwrap_or(&source.meeting.meeting_date);
    let kind = match chunk.kind.as_str() {
        "summary" => "Resumo",
        "notes" => "Notas",
        _ => "Transcrição",
    };
    format!(
        "[Projeto: {} | Reunião: \"{}\" | Data: {} | {}]\n{}",
        source.project_name, source.meeting.title, date, kind, chunk.text
    )
}

/// Indexes one meeting. Chunks are always written (so lexical search works even
/// when Ollama is down); embeddings are added when the provider is reachable.
pub async fn index_meeting(
    pool: &SqlitePool,
    embedder: &dyn EmbeddingProvider,
    meeting_id: &str,
) -> Result<IndexOutcome> {
    let _guard = INDEX_LOCK.lock().await;

    let source = load_meeting(pool, meeting_id).await?;
    let mut chunks = build_chunks(&source);

    let inputs: Vec<String> = chunks.iter().map(|c| embedding_input(&source, c)).collect();
    let embed_error = if inputs.is_empty() {
        None
    } else {
        match embedder.embed(&inputs).await {
            Ok(vectors) => {
                for (chunk, vector) in chunks.iter_mut().zip(vectors) {
                    chunk.embedding = Some(vector);
                }
                None
            }
            Err(e) => Some(e.to_string()),
        }
    };

    RagStore::replace_meeting_chunks(pool, &source.meeting, &chunks, embedder.model_id()).await?;

    let embedded_count = chunks.iter().filter(|c| c.embedding.is_some()).count();
    let status = if embed_error.is_some() { "partial" } else { "indexed" };
    RagStore::record_job(
        pool,
        meeting_id,
        status,
        chunks.len(),
        embedded_count,
        Some(embedder.model_id()),
        embed_error.as_deref(),
    )
    .await?;

    log::info!(
        "RAG: indexed meeting {} ({} chunks, {} embedded{})",
        meeting_id,
        chunks.len(),
        embedded_count,
        embed_error.as_ref().map(|e| format!(", embedding error: {e}")).unwrap_or_default()
    );

    Ok(IndexOutcome {
        meeting_id: meeting_id.to_string(),
        status: status.to_string(),
        chunk_count: chunks.len(),
        embedded_count,
        error: embed_error,
        fact_count: None,
        facts_error: None,
    })
}

/// Indexes a meeting using the app's pool and config, emitting `rag-index-progress`.
pub async fn index_meeting_with_app<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> Result<Option<IndexOutcome>> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| anyhow!("App state not available"))?;
    let pool = state.db_manager.pool().clone();

    let config = RagStore::get_config(&pool).await?;
    if !config.enabled {
        return Ok(None);
    }
    let embedder = embedder_from_config(&pool, &config).await;

    let outcome = match index_meeting(&pool, embedder.as_ref(), meeting_id).await {
        Ok(outcome) => outcome,
        Err(e) => {
            let _ = RagStore::record_job(&pool, meeting_id, "error", 0, 0, None, Some(&e.to_string())).await;
            IndexOutcome {
                meeting_id: meeting_id.to_string(),
                status: "error".into(),
                chunk_count: 0,
                embedded_count: 0,
                error: Some(e.to_string()),
                fact_count: None,
                facts_error: None,
            }
        }
    };

    let mut outcome = outcome;
    if config.extract_facts && outcome.status != "error" && outcome.chunk_count > 0 {
        let result = match ConfiguredChatModel::from_settings(&pool, app.path().app_data_dir().ok()).await {
            Ok(llm) => extract_meeting_facts(&pool, &llm, meeting_id).await,
            Err(e) => Err(e),
        };
        match result {
            Ok(count) => outcome.fact_count = Some(count),
            Err(e) => {
                log::warn!("RAG: fact extraction failed for meeting {}: {}", meeting_id, e);
                outcome.facts_error = Some(e.to_string());
            }
        }
    }
    let _ = app.emit("rag-index-progress", &outcome);
    Ok(Some(outcome))
}

/// Fire-and-forget indexing after a meeting is created or changed.
pub fn schedule_meeting_index<R: Runtime>(app: AppHandle<R>, meeting_id: String) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = index_meeting_with_app(&app, &meeting_id).await {
            log::error!("RAG: failed to index meeting {}: {}", meeting_id, e);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rag::retriever::hybrid_search;
    use crate::rag::store::SearchFilters;
    use async_trait::async_trait;
    use chrono::Utc;
    use sqlx::sqlite::SqlitePoolOptions;

    /// Deterministic bag-of-words embedder: one dimension per vocabulary word.
    struct FakeEmbedder {
        fail: bool,
    }

    const VOCAB: &[&str] = &["deploy", "bloqueado", "acesso", "orçamento", "design", "abc-123"];

    #[async_trait]
    impl EmbeddingProvider for FakeEmbedder {
        fn model_id(&self) -> &str {
            "fake"
        }
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            if self.fail {
                return Err(anyhow!("Cannot connect to Ollama"));
            }
            Ok(texts
                .iter()
                .map(|t| {
                    let lower = t.to_lowercase();
                    VOCAB.iter().map(|w| lower.matches(w).count() as f32 + 0.01).collect()
                })
                .collect())
        }
    }

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn add_meeting(pool: &SqlitePool, id: &str, title: &str, project_id: &str, lines: &[&str]) {
        let now = Utc::now();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, project_id) VALUES (?, ?, ?, ?, ?)")
            .bind(id)
            .bind(title)
            .bind(now)
            .bind(now)
            .bind(project_id)
            .execute(pool)
            .await
            .unwrap();
        for (i, line) in lines.iter().enumerate() {
            sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(format!("{id}-t{i}"))
                .bind(id)
                .bind(*line)
                .bind(format!("00:0{i}"))
                .bind(i as f64 * 60.0)
                .bind(i as f64 * 60.0 + 60.0)
                .execute(pool)
                .await
                .unwrap();
        }
    }

    async fn add_project(pool: &SqlitePool, id: &str) {
        let now = Utc::now();
        sqlx::query("INSERT INTO projects (id, name, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(id)
            .bind(id)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .unwrap();
    }

    #[test]
    fn summary_markdown_is_extracted_from_result_json() {
        assert_eq!(summary_markdown(r##"{"markdown":"# Oi"}"##).as_deref(), Some("# Oi"));
        assert_eq!(summary_markdown(r#"{"markdown":"  "}"#), None);
        assert_eq!(summary_markdown("not json"), None);
    }

    #[tokio::test]
    async fn indexes_transcript_and_summary_and_searches_hybrid() {
        let pool = test_pool().await;
        add_project(&pool, "p1").await;
        add_meeting(
            &pool,
            "m1",
            "Daily",
            "p1",
            &[
                "Bom dia pessoal",
                "O ticket ABC-123 está bloqueado por falta de acesso ao ambiente",
                "Vamos revisar o design da tela",
            ],
        )
        .await;
        sqlx::query("INSERT INTO summary_processes (meeting_id, status, created_at, updated_at, result) VALUES ('m1', 'completed', 'x', 'x', ?)")
            .bind(r##"{"markdown":"# Resumo\nDeploy adiado."}"##)
            .execute(&pool)
            .await
            .unwrap();

        let embedder = FakeEmbedder { fail: false };
        let outcome = index_meeting(&pool, &embedder, "m1").await.unwrap();
        assert_eq!(outcome.status, "indexed");
        assert!(outcome.chunk_count >= 3); // 3 one-minute segments → 2+ windows, plus summary
        assert_eq!(outcome.embedded_count, outcome.chunk_count);

        let response = hybrid_search(&pool, &embedder, "p1", "ABC-123 bloqueado", &SearchFilters::default(), 5)
            .await
            .unwrap();
        assert!(response.vector_error.is_none());
        let top = &response.results[0];
        assert!(top.hit.text.contains("ABC-123"));
        assert_eq!(top.hit.meeting_title, "Daily");
        assert!(top.sources.contains(&"lexical") && top.sources.contains(&"vector"));

        // Title matches are found lexically and follow renames
        RagStore::rename_meeting(&pool, "m1", "Planejamento trimestral").await.unwrap();
        let by_title = RagStore::lexical_search(&pool, "p1", "trimestral", &SearchFilters::default(), 5)
            .await
            .unwrap();
        assert!(!by_title.is_empty());

        let summary = hybrid_search(&pool, &embedder, "p1", "deploy", &SearchFilters::default(), 5)
            .await
            .unwrap();
        assert!(summary.results.iter().any(|r| r.hit.kind == "summary"));

        // Reindexing replaces rather than duplicates
        let again = index_meeting(&pool, &embedder, "m1").await.unwrap();
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rag_chunks WHERE meeting_id = 'm1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count as usize, again.chunk_count);
        let (fts,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rag_chunks_fts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(fts, count);
    }

    #[tokio::test]
    async fn search_is_scoped_to_project() {
        let pool = test_pool().await;
        add_project(&pool, "p1").await;
        add_project(&pool, "p2").await;
        add_meeting(&pool, "m1", "Alpha", "p1", &["orçamento aprovado"]).await;
        add_meeting(&pool, "m2", "Beta", "p2", &["orçamento negado"]).await;
        let embedder = FakeEmbedder { fail: false };
        index_meeting(&pool, &embedder, "m1").await.unwrap();
        index_meeting(&pool, &embedder, "m2").await.unwrap();

        let response = hybrid_search(&pool, &embedder, "p2", "orçamento", &SearchFilters::default(), 10)
            .await
            .unwrap();
        assert!(!response.results.is_empty());
        assert!(response.results.iter().all(|r| r.hit.meeting_id == "m2"));
    }

    #[tokio::test]
    async fn embedding_failure_keeps_lexical_search_working() {
        let pool = test_pool().await;
        add_project(&pool, "p1").await;
        add_meeting(&pool, "m1", "Daily", "p1", &["O deploy ficou bloqueado"]).await;

        let down = FakeEmbedder { fail: true };
        let outcome = index_meeting(&pool, &down, "m1").await.unwrap();
        assert_eq!(outcome.status, "partial");
        assert_eq!(outcome.embedded_count, 0);
        assert!(outcome.error.unwrap().contains("Ollama"));

        let response = hybrid_search(&pool, &down, "p1", "deploy", &SearchFilters::default(), 5)
            .await
            .unwrap();
        assert!(response.vector_error.is_some());
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].sources, vec!["lexical"]);

        let status = RagStore::project_status(&pool, "p1", "fake").await.unwrap();
        assert_eq!(status.partial_meetings, 1);
        assert_eq!(status.embedded_chunks, 0);
        assert!(status.last_error.is_some());
    }

    #[tokio::test]
    async fn index_follows_meeting_moves_and_deletes() {
        use crate::database::repositories::{meeting::MeetingsRepository, project::ProjectsRepository};
        let pool = test_pool().await;
        add_project(&pool, "p1").await;
        add_project(&pool, "p2").await;
        add_meeting(&pool, "m1", "Daily", "p1", &["deploy na sexta"]).await;
        let embedder = FakeEmbedder { fail: false };
        index_meeting(&pool, &embedder, "m1").await.unwrap();

        ProjectsRepository::set_meeting_project(&pool, "m1", "p2").await.unwrap();
        let in_p1 = hybrid_search(&pool, &embedder, "p1", "deploy", &SearchFilters::default(), 5).await.unwrap();
        let in_p2 = hybrid_search(&pool, &embedder, "p2", "deploy", &SearchFilters::default(), 5).await.unwrap();
        assert!(in_p1.results.is_empty());
        assert_eq!(in_p2.results.len(), 1);

        assert!(MeetingsRepository::delete_meeting(&pool, "m1").await.unwrap());
        let (chunks,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rag_chunks").fetch_one(&pool).await.unwrap();
        let (fts,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rag_chunks_fts").fetch_one(&pool).await.unwrap();
        assert_eq!((chunks, fts), (0, 0));
    }

    #[tokio::test]
    async fn date_and_meeting_filters_apply() {
        let pool = test_pool().await;
        add_project(&pool, "p1").await;
        add_meeting(&pool, "m1", "Daily", "p1", &["deploy hoje"]).await;
        add_meeting(&pool, "m2", "Retro", "p1", &["deploy ontem"]).await;
        let embedder = FakeEmbedder { fail: false };
        index_meeting(&pool, &embedder, "m1").await.unwrap();
        index_meeting(&pool, &embedder, "m2").await.unwrap();

        let only_m2 = SearchFilters { meeting_id: Some("m2".into()), ..Default::default() };
        let response = hybrid_search(&pool, &embedder, "p1", "deploy", &only_m2, 10).await.unwrap();
        assert!(response.results.iter().all(|r| r.hit.meeting_id == "m2"));

        let future = SearchFilters { date_from: Some("2999-01-01".into()), ..Default::default() };
        let response = hybrid_search(&pool, &embedder, "p1", "deploy", &future, 10).await.unwrap();
        assert!(response.results.is_empty());
    }
}

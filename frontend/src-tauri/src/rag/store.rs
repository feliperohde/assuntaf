//! Persistence for the RAG index: chunks + embeddings (SQLite BLOB), the FTS5
//! lexical index, per-meeting job state and the global RAG config.
//!
//! Vector search is brute-force cosine similarity in Rust, always scoped to one
//! project. At personal scale (thousands of chunks) this takes milliseconds and
//! avoids a native SQLite extension.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Connection, Error as SqlxError, FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct RagConfig {
    pub enabled: bool,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub ollama_endpoint: Option<String>,
    /// Extract tickets, decisions and action items with the summary LLM after indexing.
    pub extract_facts: bool,
    /// Detect speakers in recorded meetings before indexing.
    pub auto_diarize: bool,
}

/// A chunk ready to be written, with its embedding when one was computed.
#[derive(Debug, Clone)]
pub struct NewChunk {
    pub kind: String,
    pub chunk_index: i64,
    pub text: String,
    pub speakers: Vec<String>,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub embedding: Option<Vec<f32>>,
}

/// Where a meeting's chunks come from, for writing them.
#[derive(Debug, Clone)]
pub struct MeetingRef {
    pub meeting_id: String,
    pub project_id: String,
    pub title: String,
    pub meeting_date: String,
}

/// A chunk returned by retrieval, with the metadata needed for citations.
#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ChunkHit {
    pub chunk_id: String,
    pub meeting_id: String,
    pub meeting_title: String,
    pub meeting_date: String,
    pub kind: String,
    pub text: String,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    #[sqlx(skip)]
    pub score: f64,
}

#[derive(Debug, Clone, Default)]
pub struct SearchFilters {
    pub meeting_id: Option<String>,
    /// Inclusive lower bound on the meeting date (RFC3339 or YYYY-MM-DD prefix).
    pub date_from: Option<String>,
    /// Exclusive upper bound on the meeting date.
    pub date_to: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectIndexStatus {
    pub meeting_count: i64,
    pub indexed_meetings: i64,
    pub partial_meetings: i64,
    pub failed_meetings: i64,
    pub chunk_count: i64,
    pub embedded_chunks: i64,
    pub stale_embeddings: i64,
    pub last_error: Option<String>,
}

pub fn encode_vector(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (*x as f64, *y as f64);
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Turns free text into a safe FTS5 query: each word quoted, OR-ed together.
/// Returns `None` when the text has no searchable words.
pub fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
        .map(|t| t.trim_matches(|c| c == '-' || c == '_'))
        .filter(|t| t.chars().count() >= 2)
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

pub struct RagStore;

impl RagStore {
    pub async fn get_config(pool: &SqlitePool) -> Result<RagConfig, SqlxError> {
        sqlx::query_as::<_, RagConfig>(
            "SELECT enabled, embedding_provider, embedding_model, ollama_endpoint, extract_facts, auto_diarize FROM rag_config WHERE id = 1",
        )
        .fetch_one(pool)
        .await
    }

    pub async fn save_config(pool: &SqlitePool, config: &RagConfig) -> Result<(), SqlxError> {
        if config.embedding_model.trim().is_empty() {
            return Err(SqlxError::Protocol("embedding model cannot be empty".to_string()));
        }
        sqlx::query(
            "UPDATE rag_config SET enabled = ?, embedding_provider = ?, embedding_model = ?, ollama_endpoint = ?, extract_facts = ?, auto_diarize = ? WHERE id = 1",
        )
        .bind(config.enabled)
        .bind(&config.embedding_provider)
        .bind(config.embedding_model.trim())
        .bind(
            config
                .ollama_endpoint
                .as_deref()
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .map(|e| super::embeddings::normalize_endpoint(Some(e))),
        )
        .bind(config.extract_facts)
        .bind(config.auto_diarize)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Replaces all chunks of a meeting (rows + FTS entries) atomically.
    pub async fn replace_meeting_chunks(
        pool: &SqlitePool,
        meeting: &MeetingRef,
        chunks: &[NewChunk],
        embedding_model: &str,
    ) -> Result<(), SqlxError> {
        let mut conn = pool.acquire().await?;
        let mut tx = conn.begin().await?;

        delete_meeting_chunks_in(&mut tx, &meeting.meeting_id).await?;

        let now = Utc::now();
        for chunk in chunks {
            let id = format!("chunk-{}", Uuid::new_v4());
            let (blob, model, dims) = match &chunk.embedding {
                Some(v) => (Some(encode_vector(v)), Some(embedding_model), Some(v.len() as i64)),
                None => (None, None, None),
            };
            let speakers = if chunk.speakers.is_empty() {
                None
            } else {
                serde_json::to_string(&chunk.speakers).ok()
            };
            sqlx::query(
                "INSERT INTO rag_chunks (id, project_id, meeting_id, kind, chunk_index, text, speakers, start_time, end_time, meeting_date, embedding, embedding_model, dims, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&meeting.project_id)
            .bind(&meeting.meeting_id)
            .bind(&chunk.kind)
            .bind(chunk.chunk_index)
            .bind(&chunk.text)
            .bind(speakers)
            .bind(chunk.start_time)
            .bind(chunk.end_time)
            .bind(&meeting.meeting_date)
            .bind(blob)
            .bind(model)
            .bind(dims)
            .bind(now)
            .execute(&mut *tx)
            .await?;

            sqlx::query("INSERT INTO rag_chunks_fts (chunk_id, title, text) VALUES (?, ?, ?)")
                .bind(&id)
                .bind(&meeting.title)
                .bind(&chunk.text)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await
    }

    /// Keeps the lexical index's title column in sync after a meeting is renamed.
    /// (Embeddings keep the old title in their context header until the next reindex.)
    pub async fn rename_meeting(pool: &SqlitePool, meeting_id: &str, title: &str) -> Result<(), SqlxError> {
        sqlx::query(
            "UPDATE rag_chunks_fts SET title = ? WHERE chunk_id IN (SELECT id FROM rag_chunks WHERE meeting_id = ?)",
        )
        .bind(title)
        .bind(meeting_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn record_job(
        pool: &SqlitePool,
        meeting_id: &str,
        status: &str,
        chunk_count: usize,
        embedded_count: usize,
        embedding_model: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "INSERT INTO rag_index_jobs (meeting_id, status, chunk_count, embedded_count, embedding_model, error, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(meeting_id) DO UPDATE SET
                status = excluded.status, chunk_count = excluded.chunk_count,
                embedded_count = excluded.embedded_count, embedding_model = excluded.embedding_model,
                error = excluded.error, updated_at = excluded.updated_at",
        )
        .bind(meeting_id)
        .bind(status)
        .bind(chunk_count as i64)
        .bind(embedded_count as i64)
        .bind(embedding_model)
        .bind(error)
        .bind(Utc::now())
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn project_status(
        pool: &SqlitePool,
        project_id: &str,
        embedding_model: &str,
    ) -> Result<ProjectIndexStatus, SqlxError> {
        let (meeting_count, indexed, partial, failed): (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*),
                    COALESCE(SUM(j.status = 'indexed'), 0),
                    COALESCE(SUM(j.status = 'partial'), 0),
                    COALESCE(SUM(j.status = 'error'), 0)
             FROM meetings m LEFT JOIN rag_index_jobs j ON j.meeting_id = m.id
             WHERE m.project_id = ?",
        )
        .bind(project_id)
        .fetch_one(pool)
        .await?;

        let (chunk_count, embedded, stale): (i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*),
                    COALESCE(SUM(embedding IS NOT NULL AND embedding_model = ?), 0),
                    COALESCE(SUM(embedding IS NOT NULL AND embedding_model != ?), 0)
             FROM rag_chunks WHERE project_id = ?",
        )
        .bind(embedding_model)
        .bind(embedding_model)
        .bind(project_id)
        .fetch_one(pool)
        .await?;

        let last_error: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT j.error FROM rag_index_jobs j JOIN meetings m ON m.id = j.meeting_id
             WHERE m.project_id = ? AND j.error IS NOT NULL ORDER BY j.updated_at DESC LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(pool)
        .await?;

        Ok(ProjectIndexStatus {
            meeting_count,
            indexed_meetings: indexed,
            partial_meetings: partial,
            failed_meetings: failed,
            chunk_count,
            embedded_chunks: embedded,
            stale_embeddings: stale,
            last_error: last_error.and_then(|(e,)| e),
        })
    }

    /// Meetings of a project, oldest first, for (re)indexing.
    pub async fn project_meeting_ids(pool: &SqlitePool, project_id: &str) -> Result<Vec<String>, SqlxError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM meetings WHERE project_id = ? ORDER BY created_at")
                .bind(project_id)
                .fetch_all(pool)
                .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// Top-k chunks by cosine similarity among chunks embedded with `embedding_model`,
    /// ignoring chunks below `min_similarity` (so unrelated questions find nothing).
    pub async fn vector_search(
        pool: &SqlitePool,
        project_id: Option<&str>,
        query_vector: &[f32],
        embedding_model: &str,
        filters: &SearchFilters,
        k: usize,
        min_similarity: f64,
    ) -> Result<Vec<ChunkHit>, SqlxError> {
        let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(
            "SELECT c.id, c.embedding FROM rag_chunks c
             WHERE (? IS NULL OR c.project_id = ?) AND c.embedding IS NOT NULL AND c.embedding_model = ?
               AND (? IS NULL OR c.meeting_id = ?)
               AND (? IS NULL OR c.meeting_date >= ?)
               AND (? IS NULL OR c.meeting_date < ?)",
        )
        .bind(project_id)
        .bind(project_id)
        .bind(embedding_model)
        .bind(&filters.meeting_id)
        .bind(&filters.meeting_id)
        .bind(&filters.date_from)
        .bind(&filters.date_from)
        .bind(&filters.date_to)
        .bind(&filters.date_to)
        .fetch_all(pool)
        .await?;

        let mut scored: Vec<(String, f64)> = rows
            .into_iter()
            .map(|(id, blob)| {
                let score = cosine_similarity(query_vector, &decode_vector(&blob));
                (id, score)
            })
            .filter(|(_, score)| *score >= min_similarity)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);

        Self::hydrate(pool, scored).await
    }

    /// Top-k chunks by BM25 over chunk text and meeting title (`project_id` None = all projects).
    pub async fn lexical_search(
        pool: &SqlitePool,
        project_id: Option<&str>,
        query: &str,
        filters: &SearchFilters,
        k: usize,
    ) -> Result<Vec<ChunkHit>, SqlxError> {
        let Some(match_query) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let rows: Vec<(String, f64)> = sqlx::query_as(
            "SELECT f.chunk_id, bm25(rag_chunks_fts) AS rank
             FROM rag_chunks_fts f JOIN rag_chunks c ON c.id = f.chunk_id
             WHERE rag_chunks_fts MATCH ? AND (? IS NULL OR c.project_id = ?)
               AND (? IS NULL OR c.meeting_id = ?)
               AND (? IS NULL OR c.meeting_date >= ?)
               AND (? IS NULL OR c.meeting_date < ?)
             ORDER BY rank LIMIT ?",
        )
        .bind(&match_query)
        .bind(project_id)
        .bind(project_id)
        .bind(&filters.meeting_id)
        .bind(&filters.meeting_id)
        .bind(&filters.date_from)
        .bind(&filters.date_from)
        .bind(&filters.date_to)
        .bind(&filters.date_to)
        .bind(k as i64)
        .fetch_all(pool)
        .await?;

        // bm25() is lower-is-better; expose a positive higher-is-better score
        let scored = rows.into_iter().map(|(id, rank)| (id, -rank)).collect();
        Self::hydrate(pool, scored).await
    }

    /// Loads citation metadata for scored chunk ids, preserving their order.
    async fn hydrate(pool: &SqlitePool, scored: Vec<(String, f64)>) -> Result<Vec<ChunkHit>, SqlxError> {
        let mut hits = Vec::with_capacity(scored.len());
        for (id, score) in scored {
            let hit: Option<ChunkHit> = sqlx::query_as(
                "SELECT c.id AS chunk_id, c.meeting_id, m.title AS meeting_title, c.meeting_date,
                        c.kind, c.text, c.start_time, c.end_time
                 FROM rag_chunks c JOIN meetings m ON m.id = c.meeting_id WHERE c.id = ?",
            )
            .bind(&id)
            .fetch_optional(pool)
            .await?;
            if let Some(mut hit) = hit {
                hit.score = score;
                hits.push(hit);
            }
        }
        Ok(hits)
    }
}

async fn delete_meeting_chunks_in(
    conn: &mut sqlx::SqliteConnection,
    meeting_id: &str,
) -> Result<(), SqlxError> {
    sqlx::query(
        "DELETE FROM rag_chunks_fts WHERE chunk_id IN (SELECT id FROM rag_chunks WHERE meeting_id = ?)",
    )
    .bind(meeting_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM rag_chunks WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_roundtrip_and_cosine() {
        let v = vec![0.5f32, -1.25, 3.0];
        assert_eq!(decode_vector(&encode_vector(&v)), v);
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-9);
        assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-9);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn fts_query_quotes_terms_and_drops_noise() {
        assert_eq!(
            fts_query("Por que o ABC-123 está bloqueado?").unwrap(),
            "\"Por\" OR \"que\" OR \"ABC-123\" OR \"está\" OR \"bloqueado\""
        );
        assert_eq!(fts_query("\"a\" * ( )"), None);
    }
}

-- Migration: RAG index (phase 2 of docs/plans/projects-and-rag.md)
-- Meetings are split into chunks (transcript windows, summary sections, notes).
-- Each chunk carries project/meeting metadata, an optional embedding (f32 LE BLOB)
-- and a lexical FTS5 entry, enabling hybrid (vector + BM25) retrieval per project.

CREATE TABLE IF NOT EXISTS rag_chunks (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    meeting_id TEXT NOT NULL,
    kind TEXT NOT NULL,               -- 'transcript' | 'summary' | 'notes'
    chunk_index INTEGER NOT NULL,
    text TEXT NOT NULL,
    speakers TEXT,                    -- JSON array of speaker labels, filled by diarization
    start_time REAL,                  -- seconds from recording start
    end_time REAL,
    meeting_date TEXT NOT NULL,
    embedding BLOB,
    embedding_model TEXT,
    dims INTEGER,
    created_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_rag_chunks_project ON rag_chunks(project_id);
CREATE INDEX IF NOT EXISTS idx_rag_chunks_meeting ON rag_chunks(meeting_id);

-- Standalone FTS table kept in sync by the application (same transaction as rag_chunks)
CREATE VIRTUAL TABLE IF NOT EXISTS rag_chunks_fts USING fts5(
    chunk_id UNINDEXED,
    title,
    text,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- One row per meeting: indexing state, so failed/pending work can be retried
CREATE TABLE IF NOT EXISTS rag_index_jobs (
    meeting_id TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL,             -- 'indexed' | 'partial' | 'error'
    chunk_count INTEGER NOT NULL DEFAULT 0,
    embedded_count INTEGER NOT NULL DEFAULT 0,
    embedding_model TEXT,
    error TEXT,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

-- Global RAG configuration (single row)
CREATE TABLE IF NOT EXISTS rag_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    enabled INTEGER NOT NULL DEFAULT 1,
    embedding_provider TEXT NOT NULL DEFAULT 'ollama',
    embedding_model TEXT NOT NULL DEFAULT 'bge-m3',
    ollama_endpoint TEXT
);

INSERT OR IGNORE INTO rag_config (id) VALUES (1);

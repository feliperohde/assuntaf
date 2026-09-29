-- Migration: history of questions asked on the Ask page, per project.
-- The full answer (text, citations, plan) is kept as JSON so it can be reopened as it was.

CREATE TABLE IF NOT EXISTS ask_history (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    question TEXT NOT NULL,
    answer_json TEXT NOT NULL,
    found INTEGER NOT NULL,
    citation_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_ask_history_project_created ON ask_history(project_id, created_at);

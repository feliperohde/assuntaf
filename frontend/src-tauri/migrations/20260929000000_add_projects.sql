-- Migration: Add projects for multi-project organization
-- Every meeting belongs to a project. Projects carry context (description,
-- glossary, ticket patterns) that later feeds summaries and RAG indexing.

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    context_md TEXT,
    glossary TEXT,
    ticket_patterns TEXT,
    color TEXT,
    archived INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS project_members (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    role TEXT,
    email TEXT,
    voiceprint BLOB,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_project_members_project_id ON project_members(project_id);

-- Default project that owns all pre-existing meetings
INSERT OR IGNORE INTO projects (id, name, description, created_at, updated_at)
VALUES ('project-default', 'Geral', 'Projeto padrão', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

ALTER TABLE meetings ADD COLUMN project_id TEXT REFERENCES projects(id);

UPDATE meetings SET project_id = 'project-default' WHERE project_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_meetings_project_id ON meetings(project_id);

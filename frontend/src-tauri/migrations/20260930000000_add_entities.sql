-- Migration: entities and facts extracted from meetings (phase 4 of docs/plans/projects-and-rag.md)
-- Tickets mentioned in meetings become entities; statements about them (status,
-- blockers, decisions, action items) become dated facts pointing at the passage
-- they came from. Facts without a ticket (e.g. general decisions) have no entity.
-- `source` leaves room for future integrations (Jira, Linear) feeding the same tables.

CREATE TABLE IF NOT EXISTS entities (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    entity_type TEXT NOT NULL,        -- 'ticket'
    key TEXT NOT NULL,                -- normalized, e.g. 'ABC-123'
    display_name TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'meeting',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (project_id, entity_type, key),
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS entity_facts (
    id TEXT PRIMARY KEY NOT NULL,
    entity_id TEXT,
    project_id TEXT NOT NULL,
    meeting_id TEXT NOT NULL,
    chunk_id TEXT,                    -- evidence passage in rag_chunks
    fact_type TEXT NOT NULL,          -- 'status' | 'blocker' | 'decision' | 'action'
    content TEXT NOT NULL,
    owner TEXT,                       -- person responsible, for action items
    start_time REAL,
    meeting_date TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (entity_id) REFERENCES entities(id) ON DELETE CASCADE,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_entity_facts_entity ON entity_facts(entity_id);
CREATE INDEX IF NOT EXISTS idx_entity_facts_meeting ON entity_facts(meeting_id);
CREATE INDEX IF NOT EXISTS idx_entity_facts_project ON entity_facts(project_id);

ALTER TABLE rag_config ADD COLUMN extract_facts INTEGER NOT NULL DEFAULT 1;

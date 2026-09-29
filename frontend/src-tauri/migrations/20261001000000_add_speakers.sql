-- Migration: speaker diarization (phase 3 of docs/plans/projects-and-rag.md)
-- Each meeting gets its detected speakers ("Speaker 1", …) with a voice centroid;
-- a speaker can be linked to a project member, whose voiceprint (average of the
-- centroids assigned to them) lets future meetings recognize them automatically.

CREATE TABLE IF NOT EXISTS meeting_speakers (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL,
    label TEXT NOT NULL,
    member_id TEXT,
    centroid BLOB,
    speaking_seconds REAL NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE,
    FOREIGN KEY (member_id) REFERENCES project_members(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_meeting_speakers_meeting ON meeting_speakers(meeting_id);

ALTER TABLE transcripts ADD COLUMN speaker_id TEXT;

-- project_members.voiceprint already exists (BLOB); count of samples averaged into it
ALTER TABLE project_members ADD COLUMN voiceprint_samples INTEGER NOT NULL DEFAULT 0;

ALTER TABLE rag_config ADD COLUMN auto_diarize INTEGER NOT NULL DEFAULT 1;

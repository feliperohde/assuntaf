-- Migration: transcription on a remote OpenAI-compatible server
-- (transcript_settings.provider = 'remote' selects it).

CREATE TABLE IF NOT EXISTS remote_transcription (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    endpoint TEXT,
    api_key TEXT,
    model TEXT
);

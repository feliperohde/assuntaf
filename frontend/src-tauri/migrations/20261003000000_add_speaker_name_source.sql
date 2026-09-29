-- Migration: where a speaker's name comes from
-- name_source: 'voice' (recognized by voiceprint), 'inferred' (guessed by the LLM
-- from what was said), 'manual' (set by the user); NULL = still "Speaker N".
-- name_evidence: for inferred names, the quote that supports the guess.

ALTER TABLE meeting_speakers ADD COLUMN name_source TEXT;
ALTER TABLE meeting_speakers ADD COLUMN name_evidence TEXT;

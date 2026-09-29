-- Migration: "My time" — recognizing the app's user in meetings.
-- voice_source: which stream a transcript line came from, from the recording's
-- voice_activity.json ('mic' = the user, 'system' = others, 'both').
-- is_me: the detected speaker who is the user.
-- user_profile: the user's name and voiceprint (for recordings without a
-- mic/system track, e.g. imported audio).

ALTER TABLE transcripts ADD COLUMN voice_source TEXT;
ALTER TABLE meeting_speakers ADD COLUMN is_me INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS user_profile (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    display_name TEXT,
    voiceprint BLOB,
    voiceprint_samples INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT
);
INSERT OR IGNORE INTO user_profile (id) VALUES (1);

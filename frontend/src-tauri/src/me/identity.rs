//! Recognizing the user: transcript lines from the microphone are theirs; the
//! detected speaker who owns those lines (or whose voice matches the user's
//! voiceprint) is marked `is_me`, and the voiceprint keeps learning from it.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use std::path::Path;

use crate::audio::voice_activity::VoiceActivity;
use crate::diarization::cluster::cosine;
use crate::rag::store::{decode_vector, encode_vector};

/// Share of a speaker's labeled lines that must come from the mic to be the user.
const MIC_SPEAKER_SHARE: f64 = 0.6;
/// Minimum labeled lines to trust that share.
const MIN_LABELED_LINES: i64 = 3;
/// Voice similarity to recognize the user without a mic track.
const VOICE_MATCH_THRESHOLD: f32 = 0.6;

#[derive(Debug, Clone, Default, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    pub display_name: Option<String>,
    /// Meetings whose voice taught the voiceprint.
    pub voiceprint_samples: i64,
}

pub async fn get_profile(pool: &SqlitePool) -> Result<UserProfile> {
    Ok(sqlx::query_as("SELECT display_name, voiceprint_samples FROM user_profile WHERE id = 1")
        .fetch_optional(pool)
        .await?
        .unwrap_or_default())
}

pub async fn set_display_name(pool: &SqlitePool, name: Option<&str>) -> Result<()> {
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    sqlx::query("INSERT INTO user_profile (id, display_name, updated_at) VALUES (1, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, updated_at = excluded.updated_at")
        .bind(name)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
    // Speakers already recognized as the user carry the new name
    if let Some(name) = name {
        sqlx::query("UPDATE meeting_speakers SET label = ? WHERE is_me = 1").bind(name).execute(pool).await?;
    }
    Ok(())
}

async fn my_label(pool: &SqlitePool) -> String {
    get_profile(pool).await.ok().and_then(|p| p.display_name).unwrap_or_else(|| "Me".to_string())
}

/// Adds a centroid to the user's voiceprint (running average).
async fn learn_voice(pool: &SqlitePool, centroid: &[f32]) -> Result<()> {
    let row: Option<(Option<Vec<u8>>, i64)> =
        sqlx::query_as("SELECT voiceprint, voiceprint_samples FROM user_profile WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    let (old, samples) = row.unwrap_or((None, 0));
    let updated: Vec<f32> = match old.map(|v| decode_vector(&v)) {
        Some(old) if old.len() == centroid.len() && samples > 0 => {
            let n = samples as f32;
            old.iter().zip(centroid).map(|(o, c)| (o * n + c) / (n + 1.0)).collect()
        }
        _ => centroid.to_vec(),
    };
    sqlx::query("INSERT INTO user_profile (id, voiceprint, voiceprint_samples, updated_at) VALUES (1, ?, 1, ?)
                 ON CONFLICT(id) DO UPDATE SET voiceprint = excluded.voiceprint,
                   voiceprint_samples = user_profile.voiceprint_samples + 1, updated_at = excluded.updated_at")
        .bind(encode_vector(&updated))
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
    Ok(())
}

/// Fills `transcripts.voice_source` from the recording's mic/system track.
/// Returns the number of lines labeled (0 when the meeting has no track).
pub async fn label_voice_sources(pool: &SqlitePool, meeting_id: &str) -> Result<usize> {
    let folder: Option<(Option<String>,)> = sqlx::query_as("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await?;
    let Some(folder) = folder.and_then(|(f,)| f) else { return Ok(0) };
    let Some(track) = VoiceActivity::load(Path::new(&folder)) else { return Ok(0) };

    let lines: Vec<(String, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT id, audio_start_time, audio_end_time FROM transcripts WHERE meeting_id = ? AND voice_source IS NULL",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await?;
    let mut labeled = 0;
    for (id, start, end) in lines {
        let (Some(start), Some(end)) = (start, end) else { continue };
        if let Some(source) = track.source_of(start, end) {
            sqlx::query("UPDATE transcripts SET voice_source = ? WHERE id = ?")
                .bind(source)
                .bind(&id)
                .execute(pool)
                .await?;
            labeled += 1;
        }
    }
    Ok(labeled)
}

/// Marks which detected speaker of a meeting is the user: the one whose lines
/// mostly come from the mic, else the one whose voice matches the voiceprint.
/// A mic-based match also teaches the voiceprint. Returns the speaker id.
pub async fn recognize_me(pool: &SqlitePool, meeting_id: &str) -> Result<Option<String>> {
    label_voice_sources(pool, meeting_id).await?;

    // A choice the user made stays
    let chosen: Option<(String,)> =
        sqlx::query_as("SELECT id FROM meeting_speakers WHERE meeting_id = ? AND is_me = 1 AND name_source = 'manual'")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await?;
    if let Some((id,)) = chosen {
        return Ok(Some(id));
    }

    let stats: Vec<(String, i64, i64, Option<Vec<u8>>)> = sqlx::query_as(
        "SELECT s.id,
                SUM(CASE WHEN t.voice_source = 'mic' THEN 1 ELSE 0 END),
                SUM(CASE WHEN t.voice_source IS NOT NULL THEN 1 ELSE 0 END),
                s.centroid
         FROM meeting_speakers s LEFT JOIN transcripts t ON t.speaker_id = s.id
         WHERE s.meeting_id = ? GROUP BY s.id",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await?;

    let by_mic = stats
        .iter()
        .filter(|(_, mic, labeled, _)| *labeled >= MIN_LABELED_LINES && *mic as f64 / *labeled as f64 >= MIC_SPEAKER_SHARE)
        .max_by_key(|(_, mic, _, _)| *mic);
    let (me, learned) = match by_mic {
        Some((id, _, _, centroid)) => (Some(id.clone()), centroid.clone()),
        None => {
            let voiceprint: Option<(Option<Vec<u8>>,)> =
                sqlx::query_as("SELECT voiceprint FROM user_profile WHERE id = 1").fetch_optional(pool).await?;
            let voiceprint = voiceprint.and_then(|(v,)| v).map(|v| decode_vector(&v));
            let best = voiceprint.and_then(|vp| {
                stats
                    .iter()
                    .filter_map(|(id, _, _, c)| c.as_ref().map(|c| (id, cosine(&decode_vector(c), &vp))))
                    .filter(|(_, sim)| *sim >= VOICE_MATCH_THRESHOLD)
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(id, _)| id.clone())
            });
            (best, None)
        }
    };

    sqlx::query("UPDATE meeting_speakers SET is_me = 0 WHERE meeting_id = ? AND is_me = 1")
        .bind(meeting_id)
        .execute(pool)
        .await?;
    if let Some(id) = &me {
        let label = my_label(pool).await;
        sqlx::query("UPDATE meeting_speakers SET is_me = 1, label = ?, name_source = 'voice', name_evidence = NULL WHERE id = ?")
            .bind(&label)
            .bind(id)
            .execute(pool)
            .await?;
        if let Some(centroid) = learned {
            learn_voice(pool, &decode_vector(&centroid)).await?;
        }
    }
    Ok(me)
}

/// The user says a detected speaker is (or is not) them; teaches the voiceprint.
pub async fn set_me(pool: &SqlitePool, speaker_id: &str, is_me: bool) -> Result<String> {
    let row: (String, Option<Vec<u8>>) = sqlx::query_as("SELECT meeting_id, centroid FROM meeting_speakers WHERE id = ?")
        .bind(speaker_id)
        .fetch_one(pool)
        .await?;
    let (meeting_id, centroid) = row;
    if is_me {
        sqlx::query("UPDATE meeting_speakers SET is_me = 0 WHERE meeting_id = ?")
            .bind(&meeting_id)
            .execute(pool)
            .await?;
        let label = my_label(pool).await;
        sqlx::query("UPDATE meeting_speakers SET is_me = 1, label = ?, name_source = 'manual', name_evidence = NULL WHERE id = ?")
            .bind(&label)
            .bind(speaker_id)
            .execute(pool)
            .await?;
        if let Some(centroid) = centroid {
            learn_voice(pool, &decode_vector(&centroid)).await?;
        }
    } else {
        sqlx::query("UPDATE meeting_speakers SET is_me = 0, name_source = 'manual' WHERE id = ?")
            .bind(speaker_id)
            .execute(pool)
            .await?;
    }
    Ok(meeting_id)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::diarization::store::{NewSpeaker, SpeakerStore};
    use sqlx::sqlite::SqlitePoolOptions;

    pub(crate) async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// Meeting with a folder holding a voice track: 0–10 s user (mic), 10–20 s others.
    pub(crate) async fn meeting_with_track(pool: &SqlitePool, id: &str, project: &str, date: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("assunta-me-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder).unwrap();
        let mut track = VoiceActivity::new();
        let loud: Vec<f32> = (0..1600).map(|i| 0.3 * ((i as f32) * 0.1).sin()).collect();
        let faint: Vec<f32> = (0..1600).map(|i| 0.01 * ((i as f32) * 0.1).sin()).collect();
        for _ in 0..100 {
            track.push_window(&loud, &faint, 16_000);
        }
        for _ in 0..100 {
            track.push_window(&faint, &loud, 16_000);
        }
        track.save(&folder).unwrap();
        sqlx::query("INSERT OR IGNORE INTO projects (id, name, created_at, updated_at) VALUES (?, ?, 'x', 'x')")
            .bind(project)
            .bind(format!("Projeto {project}"))
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, project_id, folder_path) VALUES (?, 'Daily', ?, ?, ?, ?)")
            .bind(id)
            .bind(date)
            .bind(date)
            .bind(project)
            .bind(folder.to_string_lossy().to_string())
            .execute(pool)
            .await
            .unwrap();
        folder
    }

    pub(crate) async fn line(pool: &SqlitePool, meeting: &str, n: usize, start: f64, text: &str) {
        sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration) VALUES (?, ?, ?, '0', ?, ?, 2.0)")
            .bind(format!("{meeting}-t{n}"))
            .bind(meeting)
            .bind(text)
            .bind(start)
            .bind(start + 2.0)
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn mic_lines_mark_the_speaker_as_me_and_teach_the_voice() {
        let pool = pool().await;
        set_display_name(&pool, Some("Felipe")).await.unwrap();
        meeting_with_track(&pool, "m1", "p1", "2026-09-22 10:00:00+00:00").await;
        for (n, start) in [(0, 1.0), (1, 4.0), (2, 7.0), (3, 12.0), (4, 15.0)] {
            line(&pool, "m1", n, start, "texto").await;
        }
        assert_eq!(label_voice_sources(&pool, "m1").await.unwrap(), 5);
        let sources: Vec<(String,)> = sqlx::query_as("SELECT voice_source FROM transcripts ORDER BY audio_start_time")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(sources.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), vec!["mic", "mic", "mic", "system", "system"]);

        let speakers = vec![
            NewSpeaker { centroid: vec![1.0, 0.0], speaking_seconds: 9.0 },
            NewSpeaker { centroid: vec![0.0, 1.0], speaking_seconds: 6.0 },
        ];
        let assignments = [(0, 0), (1, 0), (2, 0), (3, 1), (4, 1)].map(|(t, s)| (format!("m1-t{t}"), Some(s)));
        let saved = SpeakerStore::save(&pool, "m1", &speakers, &assignments).await.unwrap();
        assert_eq!(recognize_me(&pool, "m1").await.unwrap(), Some(saved[0].id.clone()));
        let listed = SpeakerStore::list(&pool, "m1").await.unwrap();
        assert!(listed[0].is_me && listed[0].label == "Felipe" && !listed[1].is_me);
        assert_eq!(get_profile(&pool).await.unwrap().voiceprint_samples, 1);

        // A meeting without mic track: recognized by the learned voice
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, project_id) VALUES ('m2', 'Import', 'x', 'x', 'p1')")
            .execute(&pool).await.unwrap();
        line(&pool, "m2", 0, 1.0, "oi").await;
        let other = vec![
            NewSpeaker { centroid: vec![0.1, 1.0], speaking_seconds: 5.0 },
            NewSpeaker { centroid: vec![0.95, 0.1], speaking_seconds: 5.0 },
        ];
        let saved = SpeakerStore::save(&pool, "m2", &other, &[("m2-t0".into(), Some(0))]).await.unwrap();
        assert_eq!(recognize_me(&pool, "m2").await.unwrap(), Some(saved[1].id.clone()));

        // The user corrects it
        set_me(&pool, &saved[0].id, true).await.unwrap();
        let listed = SpeakerStore::list(&pool, "m2").await.unwrap();
        assert!(listed[0].is_me && !listed[1].is_me);
        assert_eq!(recognize_me(&pool, "m2").await.unwrap(), Some(saved[0].id.clone())); // stays
    }
}

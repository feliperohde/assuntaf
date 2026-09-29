//! Persistence of diarization results and speaker ↔ member identification.

use chrono::Utc;
use serde::Serialize;
use sqlx::{Connection, FromRow, SqlitePool};
use uuid::Uuid;

use super::cluster::cosine;
use crate::rag::store::{decode_vector, encode_vector};

/// Minimum voice similarity to recognize a project member automatically.
pub const MEMBER_MATCH_THRESHOLD: f32 = 0.6;

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSpeaker {
    pub id: String,
    pub label: String,
    pub member_id: Option<String>,
    pub member_name: Option<String>,
    pub speaking_seconds: f64,
    /// 'voice', 'inferred' or 'manual'; None while the speaker is unnamed.
    pub name_source: Option<String>,
    pub name_evidence: Option<String>,
    /// The app's user (recognized by microphone or voice, or set by them).
    pub is_me: bool,
}

impl MeetingSpeaker {
    pub fn display_name(&self) -> &str {
        self.member_name.as_deref().unwrap_or(&self.label)
    }
}

/// One detected speaker ready to be stored.
pub struct NewSpeaker {
    pub centroid: Vec<f32>,
    pub speaking_seconds: f64,
}

/// Greedy one-to-one matching of speakers to members by voice similarity:
/// the most similar (speaker, member) pair above the threshold is taken first.
pub fn match_members(
    centroids: &[Vec<f32>],
    voiceprints: &[(String, Vec<f32>)],
    threshold: f32,
) -> Vec<Option<String>> {
    let mut pairs: Vec<(usize, usize, f32)> = Vec::new();
    for (s, centroid) in centroids.iter().enumerate() {
        for (m, (_, voiceprint)) in voiceprints.iter().enumerate() {
            let similarity = cosine(centroid, voiceprint);
            if similarity >= threshold {
                pairs.push((s, m, similarity));
            }
        }
    }
    pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let mut result = vec![None; centroids.len()];
    let mut used_members = Vec::new();
    for (s, m, _) in pairs {
        if result[s].is_none() && !used_members.contains(&m) {
            result[s] = Some(voiceprints[m].0.clone());
            used_members.push(m);
        }
    }
    result
}

pub struct SpeakerStore;

impl SpeakerStore {
    pub async fn has_speakers(pool: &SqlitePool, meeting_id: &str) -> Result<bool, sqlx::Error> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM meeting_speakers WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_one(pool)
            .await?;
        Ok(count > 0)
    }

    /// Replaces a meeting's speakers and per-segment assignments. Speakers whose
    /// voice matches a member of the meeting's project are linked to that member.
    /// `segment_speakers` pairs transcript ids with an index into `speakers`.
    pub async fn save(
        pool: &SqlitePool,
        meeting_id: &str,
        speakers: &[NewSpeaker],
        segment_speakers: &[(String, Option<usize>)],
    ) -> Result<Vec<MeetingSpeaker>, sqlx::Error> {
        let voiceprints: Vec<(String, Option<Vec<u8>>)> = sqlx::query_as(
            "SELECT pm.id, pm.voiceprint FROM project_members pm
             JOIN meetings m ON m.project_id = pm.project_id
             WHERE m.id = ? AND pm.voiceprint IS NOT NULL",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        let voiceprints: Vec<(String, Vec<f32>)> = voiceprints
            .into_iter()
            .filter_map(|(id, blob)| blob.map(|b| (id, decode_vector(&b))))
            .collect();
        let centroids: Vec<Vec<f32>> = speakers.iter().map(|s| s.centroid.clone()).collect();
        let members = match_members(&centroids, &voiceprints, MEMBER_MATCH_THRESHOLD);

        let mut conn = pool.acquire().await?;
        let mut tx = conn.begin().await?;
        sqlx::query("UPDATE transcripts SET speaker_id = NULL WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM meeting_speakers WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *tx)
            .await?;

        let now = Utc::now();
        let mut ids = Vec::with_capacity(speakers.len());
        for (i, speaker) in speakers.iter().enumerate() {
            let id = format!("speaker-{}", Uuid::new_v4());
            sqlx::query(
                "INSERT INTO meeting_speakers (id, meeting_id, label, member_id, centroid, speaking_seconds, created_at, name_source)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(meeting_id)
            .bind(format!("Speaker {}", i + 1))
            .bind(&members[i])
            .bind(encode_vector(&speaker.centroid))
            .bind(speaker.speaking_seconds)
            .bind(now)
            .bind(members[i].as_ref().map(|_| "voice"))
            .execute(&mut *tx)
            .await?;
            ids.push(id);
        }
        for (transcript_id, speaker) in segment_speakers {
            if let Some(index) = speaker.filter(|&i| i < ids.len()) {
                sqlx::query("UPDATE transcripts SET speaker_id = ? WHERE id = ? AND meeting_id = ?")
                    .bind(&ids[index])
                    .bind(transcript_id)
                    .bind(meeting_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        drop(conn); // release before querying again (small pools)
        Self::list(pool, meeting_id).await
    }

    pub async fn list(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<MeetingSpeaker>, sqlx::Error> {
        sqlx::query_as(
            "SELECT s.id, s.label, s.member_id, pm.name AS member_name, s.speaking_seconds,
                    s.name_source, s.name_evidence, s.is_me
             FROM meeting_speakers s LEFT JOIN project_members pm ON pm.id = s.member_id
             WHERE s.meeting_id = ? ORDER BY s.rowid",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    /// Renames a speaker and/or links it to a project member (the user's choice,
    /// so the name becomes 'manual'). Linking also teaches the member's voiceprint
    /// (running average of assigned centroids), as does confirming a link that
    /// was only inferred. Returns the speaker's meeting id.
    pub async fn update(
        pool: &SqlitePool,
        speaker_id: &str,
        label: Option<&str>,
        member_id: Option<&str>,
    ) -> Result<String, sqlx::Error> {
        let row: Option<(String, Option<Vec<u8>>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT meeting_id, centroid, member_id, name_source FROM meeting_speakers WHERE id = ?",
        )
        .bind(speaker_id)
        .fetch_optional(pool)
        .await?;
        let (meeting_id, centroid, previous_member, previous_source) = row.ok_or(sqlx::Error::RowNotFound)?;

        let mut conn = pool.acquire().await?;
        let mut tx = conn.begin().await?;
        if let Some(label) = label.map(str::trim).filter(|l| !l.is_empty()) {
            sqlx::query("UPDATE meeting_speakers SET label = ? WHERE id = ?")
                .bind(label)
                .bind(speaker_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE meeting_speakers SET member_id = ?, name_source = 'manual', name_evidence = NULL WHERE id = ?")
            .bind(member_id)
            .bind(speaker_id)
            .execute(&mut *tx)
            .await?;

        let confirmed_guess = previous_source.as_deref() == Some("inferred");
        let newly_linked = member_id.filter(|m| previous_member.as_deref() != Some(*m) || confirmed_guess);
        if let (Some(member_id), Some(centroid)) = (newly_linked, centroid) {
            let member: Option<(Option<Vec<u8>>, i64)> =
                sqlx::query_as("SELECT voiceprint, voiceprint_samples FROM project_members WHERE id = ?")
                    .bind(member_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let (voiceprint, samples) = member.ok_or(sqlx::Error::RowNotFound)?;
            let centroid = decode_vector(&centroid);
            let updated: Vec<f32> = match voiceprint.map(|v| decode_vector(&v)) {
                Some(old) if old.len() == centroid.len() && samples > 0 => {
                    let n = samples as f32;
                    old.iter().zip(&centroid).map(|(o, c)| (o * n + c) / (n + 1.0)).collect()
                }
                _ => centroid,
            };
            sqlx::query("UPDATE project_members SET voiceprint = ?, voiceprint_samples = voiceprint_samples + 1 WHERE id = ?")
                .bind(encode_vector(&updated))
                .bind(member_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(meeting_id)
    }
    /// Names a speaker from an LLM guess. Never overrides a name the user set or
    /// a voice recognition; does not touch voiceprints (a guess is not proof).
    pub async fn apply_inferred_name(
        pool: &SqlitePool,
        speaker_id: &str,
        name: &str,
        member_id: Option<&str>,
        evidence: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE meeting_speakers SET label = ?, member_id = ?, name_source = 'inferred', name_evidence = ?
             WHERE id = ? AND (name_source IS NULL OR name_source = 'inferred')",
        )
        .bind(name)
        .bind(member_id)
        .bind(evidence)
        .bind(speaker_id)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Folds speaker `from` into `into` (same meeting): their transcript lines,
    /// speaking time and voice centroid (weighted by speaking time).
    pub async fn merge(pool: &SqlitePool, into: &str, from: &str) -> Result<String, sqlx::Error> {
        type Row = (String, Option<Vec<u8>>, f64);
        let load = |id: &str| {
            sqlx::query_as::<_, Row>("SELECT meeting_id, centroid, speaking_seconds FROM meeting_speakers WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(pool)
        };
        let (meeting_id, into_centroid, into_seconds) = load(into).await?.ok_or(sqlx::Error::RowNotFound)?;
        let (from_meeting, from_centroid, from_seconds) = load(from).await?.ok_or(sqlx::Error::RowNotFound)?;
        if from_meeting != meeting_id || into == from {
            return Err(sqlx::Error::Protocol("speakers must be two different speakers of the same meeting".into()));
        }
        let centroid = match (into_centroid.map(|c| decode_vector(&c)), from_centroid.map(|c| decode_vector(&c))) {
            (Some(a), Some(b)) if a.len() == b.len() => {
                let (wa, wb) = (into_seconds.max(0.1) as f32, from_seconds.max(0.1) as f32);
                Some(a.iter().zip(&b).map(|(x, y)| (x * wa + y * wb) / (wa + wb)).collect::<Vec<f32>>())
            }
            (a, b) => a.or(b),
        };

        let mut conn = pool.acquire().await?;
        let mut tx = conn.begin().await?;
        sqlx::query("UPDATE transcripts SET speaker_id = ? WHERE speaker_id = ? AND meeting_id = ?")
            .bind(into)
            .bind(from)
            .bind(&meeting_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE meeting_speakers SET speaking_seconds = ?, centroid = ? WHERE id = ?")
            .bind(into_seconds + from_seconds)
            .bind(centroid.map(|c| encode_vector(&c)))
            .bind(into)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM meeting_speakers WHERE id = ?").bind(from).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(meeting_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn members_are_matched_one_to_one_by_best_similarity() {
        let centroids = vec![vec![1.0, 0.0], vec![0.9, 0.1], vec![0.0, 1.0]];
        let voiceprints = vec![("ana".to_string(), vec![1.0, 0.0]), ("bia".to_string(), vec![0.0, 1.0])];
        let matched = match_members(&centroids, &voiceprints, 0.6);
        // Speaker 0 is the closest to Ana; speaker 1 would also match Ana but she is taken
        assert_eq!(matched, vec![Some("ana".into()), None, Some("bia".into())]);
        assert_eq!(match_members(&centroids, &[], 0.6), vec![None, None, None]);
    }

    async fn setup() -> SqlitePool {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name, created_at, updated_at) VALUES ('p1', 'P', 'x', 'x')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO project_members (id, project_id, name, created_at, updated_at) VALUES ('ana', 'p1', 'Ana', 'x', 'x')")
            .execute(&pool).await.unwrap();
        for m in ["m1", "m2"] {
            sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, project_id) VALUES (?, 'Daily', 'x', 'x', 'p1')")
                .bind(m).execute(&pool).await.unwrap();
            for t in 0..2 {
                sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES (?, ?, 'oi', '0')")
                    .bind(format!("{m}-t{t}")).bind(m).execute(&pool).await.unwrap();
            }
        }
        pool
    }

    #[tokio::test]
    async fn assigning_a_member_teaches_their_voice_for_later_meetings() {
        let pool = setup().await;
        let speakers = vec![
            NewSpeaker { centroid: vec![1.0, 0.0], speaking_seconds: 30.0 },
            NewSpeaker { centroid: vec![0.0, 1.0], speaking_seconds: 10.0 },
        ];
        let saved = SpeakerStore::save(&pool, "m1", &speakers, &[("m1-t0".into(), Some(0)), ("m1-t1".into(), Some(1))])
            .await
            .unwrap();
        assert_eq!(saved.len(), 2);
        assert!(saved.iter().all(|s| s.member_id.is_none())); // no voiceprints yet
        assert_eq!(saved[0].display_name(), "Speaker 1");

        // The user says Speaker 1 is Ana → her voiceprint is learned
        let meeting = SpeakerStore::update(&pool, &saved[0].id, None, Some("ana")).await.unwrap();
        assert_eq!(meeting, "m1");
        let listed = SpeakerStore::list(&pool, "m1").await.unwrap();
        assert_eq!(listed[0].display_name(), "Ana");

        // Next meeting: a similar voice is recognized as Ana automatically
        let next = vec![
            NewSpeaker { centroid: vec![0.1, 1.0], speaking_seconds: 5.0 },
            NewSpeaker { centroid: vec![0.95, 0.05], speaking_seconds: 20.0 },
        ];
        let saved = SpeakerStore::save(&pool, "m2", &next, &[("m2-t0".into(), Some(1))]).await.unwrap();
        assert_eq!(saved[0].member_id, None);
        assert_eq!(saved[1].member_name.as_deref(), Some("Ana"));
        let (speaker_id,): (Option<String>,) = sqlx::query_as("SELECT speaker_id FROM transcripts WHERE id = 'm2-t0'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(speaker_id, Some(saved[1].id.clone()));

        // Renaming keeps the member link; re-saving replaces previous speakers
        SpeakerStore::update(&pool, &saved[0].id, Some("Convidado"), None).await.unwrap();
        assert_eq!(SpeakerStore::list(&pool, "m2").await.unwrap()[0].label, "Convidado");
        let (samples,): (i64,) = sqlx::query_as("SELECT voiceprint_samples FROM project_members WHERE id = 'ana'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(samples, 1);
    }
}

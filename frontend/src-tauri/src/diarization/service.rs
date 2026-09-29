//! Orchestrates diarization of a saved meeting: locate its audio, make sure the
//! models are present, diarize off the async runtime, map speakers onto the
//! transcript segments and store the result.

use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use serde::Serialize;
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

use super::cluster::assign_speakers;
use super::engine::{diarize, ensure_models, models_dir};
use super::store::{MeetingSpeaker, NewSpeaker, SpeakerStore};
use crate::audio::decoder::decode_audio_file;
use crate::audio::retranscription::find_audio_file;

/// A transcript segment with no overlapping speech gets the nearest speaker within this distance.
const MAX_ASSIGN_DISTANCE_SECONDS: f64 = 2.0;

/// Diarization is CPU-heavy; run one at a time.
static DIARIZE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizeOutcome {
    pub meeting_id: String,
    pub speakers: Vec<MeetingSpeaker>,
    /// Transcript segments that received a speaker.
    pub assigned_segments: usize,
    pub total_segments: usize,
}

async fn meeting_audio(pool: &SqlitePool, meeting_id: &str) -> Result<PathBuf> {
    let folder: Option<(Option<String>,)> = sqlx::query_as("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await?;
    let folder = folder
        .ok_or_else(|| anyhow!("Meeting {} not found", meeting_id))?
        .0
        .ok_or_else(|| anyhow!("Meeting has no recording folder"))?;
    find_audio_file(Path::new(&folder))
}

/// Whether the meeting has a recording that diarization could use.
pub async fn has_audio(pool: &SqlitePool, meeting_id: &str) -> bool {
    meeting_audio(pool, meeting_id).await.is_ok()
}

pub async fn diarize_meeting(pool: &SqlitePool, app_data_dir: &Path, meeting_id: &str) -> Result<DiarizeOutcome> {
    let _guard = DIARIZE_LOCK.lock().await;

    let audio_path = meeting_audio(pool, meeting_id).await?;
    let models = models_dir(app_data_dir);
    ensure_models(&models).await?;

    log::info!("Diarization: processing {}", audio_path.display());
    let result = tokio::task::spawn_blocking(move || -> Result<_> {
        let decoded = decode_audio_file(&audio_path)?;
        let audio = decoded.to_whisper_format();
        diarize(&audio, &models)
    })
    .await
    .map_err(|e| anyhow!("Diarization task failed: {e}"))??;

    let segments: Vec<(String, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT id, audio_start_time, audio_end_time FROM transcripts WHERE meeting_id = ?",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await?;
    let times: Vec<(Option<f64>, Option<f64>)> = segments.iter().map(|(_, s, e)| (*s, *e)).collect();
    let assigned = assign_speakers(&times, &result.turns, MAX_ASSIGN_DISTANCE_SECONDS);
    let segment_speakers: Vec<(String, Option<usize>)> = segments
        .iter()
        .zip(assigned)
        .map(|((id, _, _), speaker)| (id.clone(), speaker))
        .collect();

    let speakers: Vec<NewSpeaker> = result
        .centroids
        .iter()
        .zip(&result.speaking_seconds)
        .map(|(centroid, seconds)| NewSpeaker { centroid: centroid.clone(), speaking_seconds: *seconds })
        .collect();
    let stored = SpeakerStore::save(pool, meeting_id, &speakers, &segment_speakers).await?;

    let assigned_segments = segment_speakers.iter().filter(|(_, s)| s.is_some()).count();
    log::info!(
        "Diarization: meeting {} → {} speakers, {}/{} segments assigned",
        meeting_id,
        stored.len(),
        assigned_segments,
        segment_speakers.len()
    );
    Ok(DiarizeOutcome {
        meeting_id: meeting_id.to_string(),
        speakers: stored,
        assigned_segments,
        total_segments: segment_speakers.len(),
    })
}

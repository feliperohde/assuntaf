use log::error;
use tauri::{AppHandle, Manager, Runtime};

use super::engine::DiarizeOptions;
use super::naming::NamingOutcome;
use super::service::{diarize_meeting as run_diarization, name_speakers, DiarizeOutcome};
use super::store::{MeetingSpeaker, SpeakerStore};
use crate::rag::indexer::{schedule_meeting_index_with, IndexOptions};
use crate::state::AppState;

fn err(action: &str, e: impl std::fmt::Display) -> String {
    error!("Diarization: failed to {}: {}", action, e);
    format!("Failed to {}: {}", action, e)
}

/// Detects who spoke when in a meeting's recording (downloads the models on first
/// use; `num_speakers` pins the number of people when the user knows it
/// use), then reindexes the meeting so passages carry speaker names.
#[tauri::command]
pub async fn diarize_meeting<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    num_speakers: Option<usize>,
) -> Result<DiarizeOutcome, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| err("locate app data", e))?;
    let options = DiarizeOptions { num_speakers: num_speakers.filter(|&n| n > 0), ..Default::default() };
    let outcome = run_diarization(state.db_manager.pool(), &app_data_dir, &meeting_id, options)
        .await
        .map_err(|e| err("detect speakers", e))?;
    schedule_meeting_index_with(app, meeting_id, IndexOptions { diarize: false, extract_facts: true });
    Ok(outcome)
}

#[tauri::command]
pub async fn list_meeting_speakers(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<MeetingSpeaker>, String> {
    SpeakerStore::list(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| err("list speakers", e))
}

/// Renames a speaker and/or links it to a project member (`member_id` null unlinks).
/// Linking teaches the member's voice for automatic recognition in later meetings.
#[tauri::command]
pub async fn update_meeting_speaker<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    speaker_id: String,
    label: Option<String>,
    member_id: Option<String>,
) -> Result<Vec<MeetingSpeaker>, String> {
    let pool = state.db_manager.pool();
    let meeting_id = SpeakerStore::update(pool, &speaker_id, label.as_deref(), member_id.as_deref())
        .await
        .map_err(|e| err("update speaker", e))?;
    // Only speaker names changed: refresh passages, skip re-extracting facts
    schedule_meeting_index_with(app, meeting_id.clone(), IndexOptions { diarize: false, extract_facts: false });
    SpeakerStore::list(pool, &meeting_id).await.map_err(|e| err("list speakers", e))
}

/// Guesses unnamed speakers' names from the transcript (summary LLM) and merges
/// labels that turn out to be the same person.
#[tauri::command]
pub async fn infer_speaker_names<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<NamingOutcome, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| err("locate app data", e))?;
    let outcome = name_speakers(state.db_manager.pool(), &app_data_dir, &meeting_id)
        .await
        .map_err(|e| err("infer speaker names", e))?;
    if outcome.named + outcome.merged > 0 {
        schedule_meeting_index_with(app, meeting_id, IndexOptions { diarize: false, extract_facts: false });
    }
    Ok(outcome)
}

/// Folds speaker `from_id` into `into_id` (one person detected as two).
#[tauri::command]
pub async fn merge_meeting_speakers<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    into_id: String,
    from_id: String,
) -> Result<Vec<MeetingSpeaker>, String> {
    let pool = state.db_manager.pool();
    let meeting_id = SpeakerStore::merge(pool, &into_id, &from_id)
        .await
        .map_err(|e| err("merge speakers", e))?;
    schedule_meeting_index_with(app, meeting_id.clone(), IndexOptions { diarize: false, extract_facts: false });
    SpeakerStore::list(pool, &meeting_id).await.map_err(|e| err("list speakers", e))
}

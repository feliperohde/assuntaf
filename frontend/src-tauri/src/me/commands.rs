use log::error;
use tauri::{AppHandle, Manager, Runtime};

use super::identity::{get_profile, set_display_name, set_me, UserProfile};
use super::report::{build_report, MyTimeReport, MyTimeRequest};
use crate::diarization::store::{MeetingSpeaker, SpeakerStore};
use crate::rag::indexer::{schedule_meeting_index_with, IndexOptions};
use crate::rag::llm::ConfiguredChatModel;
use crate::state::AppState;

fn err(action: &str, e: impl std::fmt::Display) -> String {
    error!("Me: failed to {}: {}", action, e);
    format!("Failed to {}: {}", action, e)
}

#[tauri::command]
pub async fn get_user_profile(state: tauri::State<'_, AppState>) -> Result<UserProfile, String> {
    get_profile(state.db_manager.pool()).await.map_err(|e| err("load profile", e))
}

#[tauri::command]
pub async fn set_user_display_name(
    state: tauri::State<'_, AppState>,
    name: Option<String>,
) -> Result<UserProfile, String> {
    let pool = state.db_manager.pool();
    set_display_name(pool, name.as_deref()).await.map_err(|e| err("save name", e))?;
    get_profile(pool).await.map_err(|e| err("load profile", e))
}

/// "This is me" / "not me" on a detected speaker; teaches the user's voice.
#[tauri::command]
pub async fn set_meeting_speaker_is_me<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    speaker_id: String,
    is_me: bool,
) -> Result<Vec<MeetingSpeaker>, String> {
    let pool = state.db_manager.pool();
    let meeting_id = set_me(pool, &speaker_id, is_me).await.map_err(|e| err("update speaker", e))?;
    schedule_meeting_index_with(app, meeting_id.clone(), IndexOptions { diarize: false, extract_facts: false });
    SpeakerStore::list(pool, &meeting_id).await.map_err(|e| err("list speakers", e))
}

/// Tickets the user worked on in a period, with descriptions and time estimates.
#[tauri::command]
pub async fn my_time_report<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    request: MyTimeRequest,
) -> Result<MyTimeReport, String> {
    let pool = state.db_manager.pool();
    let llm = ConfiguredChatModel::from_settings(pool, app.path().app_data_dir().ok()).await.ok();
    build_report(pool, llm.as_ref().map(|m| m as &dyn crate::rag::llm::ChatModel), &request)
        .await
        .map_err(|e| err("build the report", e))
}

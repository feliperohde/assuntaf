use log::{error, info};
use serde::Deserialize;
use tauri::{AppHandle, Runtime};

use super::answer::{ask_project, Answer, ConversationTurn};
use super::indexer::{embedder_from_config, index_meeting_with_app, IndexOutcome};
use super::llm::ConfiguredChatModel;
use super::retriever::{hybrid_search, SearchResponse};
use super::store::{ProjectIndexStatus, RagConfig, RagStore, SearchFilters};
use crate::state::AppState;

const DEFAULT_SEARCH_LIMIT: usize = 10;
const MAX_SEARCH_LIMIT: usize = 50;

fn err(action: &str, e: impl std::fmt::Display) -> String {
    error!("RAG: failed to {}: {}", action, e);
    format!("Failed to {}: {}", action, e)
}

#[tauri::command]
pub async fn rag_get_config(state: tauri::State<'_, AppState>) -> Result<RagConfig, String> {
    RagStore::get_config(state.db_manager.pool())
        .await
        .map_err(|e| err("load RAG config", e))
}

#[tauri::command]
pub async fn rag_save_config(
    state: tauri::State<'_, AppState>,
    config: RagConfig,
) -> Result<RagConfig, String> {
    let pool = state.db_manager.pool();
    RagStore::save_config(pool, &config)
        .await
        .map_err(|e| err("save RAG config", e))?;
    RagStore::get_config(pool).await.map_err(|e| err("load RAG config", e))
}

#[tauri::command]
pub async fn rag_index_status(
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> Result<ProjectIndexStatus, String> {
    let pool = state.db_manager.pool();
    let config = RagStore::get_config(pool).await.map_err(|e| err("load RAG config", e))?;
    RagStore::project_status(pool, &project_id, &config.embedding_model)
        .await
        .map_err(|e| err("load index status", e))
}

#[tauri::command]
pub async fn rag_index_meeting<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<Option<IndexOutcome>, String> {
    index_meeting_with_app(&app, &meeting_id)
        .await
        .map_err(|e| err("index meeting", e))
}

/// Reindexes every meeting of a project in the background; progress arrives as
/// `rag-index-progress` events. Returns the number of meetings queued.
#[tauri::command]
pub async fn rag_reindex_project<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> Result<usize, String> {
    let meeting_ids = RagStore::project_meeting_ids(state.db_manager.pool(), &project_id)
        .await
        .map_err(|e| err("list project meetings", e))?;
    let count = meeting_ids.len();
    info!("RAG: reindexing {} meetings of project {}", count, project_id);
    tauri::async_runtime::spawn(async move {
        for meeting_id in meeting_ids {
            if let Err(e) = index_meeting_with_app(&app, &meeting_id).await {
                error!("RAG: failed to index meeting {}: {}", meeting_id, e);
            }
        }
    });
    Ok(count)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub project_id: String,
    pub query: String,
    pub meeting_id: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub limit: Option<usize>,
}

#[tauri::command]
pub async fn rag_search(
    state: tauri::State<'_, AppState>,
    request: SearchRequest,
) -> Result<SearchResponse, String> {
    let pool = state.db_manager.pool();
    if request.query.trim().is_empty() {
        return Ok(SearchResponse { results: Vec::new(), vector_error: None });
    }
    let config = RagStore::get_config(pool).await.map_err(|e| err("load RAG config", e))?;
    let embedder = embedder_from_config(pool, &config).await;
    let filters = SearchFilters {
        meeting_id: request.meeting_id,
        date_from: request.date_from,
        date_to: request.date_to,
    };
    let limit = request.limit.unwrap_or(DEFAULT_SEARCH_LIMIT).clamp(1, MAX_SEARCH_LIMIT);
    hybrid_search(pool, embedder.as_ref(), &request.project_id, request.query.trim(), &filters, limit)
        .await
        .map_err(|e| err("search", e))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskRequest {
    pub project_id: String,
    pub question: String,
    #[serde(default)]
    pub history: Vec<ConversationTurn>,
}

/// Answers a question about a project's meetings, citing the passages used.
#[tauri::command]
pub async fn rag_ask<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    request: AskRequest,
) -> Result<Answer, String> {
    use tauri::Manager;

    let pool = state.db_manager.pool();
    if request.question.trim().is_empty() {
        return Err("Question cannot be empty".to_string());
    }
    let config = RagStore::get_config(pool).await.map_err(|e| err("load RAG config", e))?;
    let embedder = embedder_from_config(pool, &config).await;
    let llm = ConfiguredChatModel::from_settings(pool, app.path().app_data_dir().ok())
        .await
        .map_err(|e| err("prepare the language model", e))?;
    info!("RAG: answering question for project {}", request.project_id);
    ask_project(
        pool,
        embedder.as_ref(),
        &llm,
        &request.project_id,
        &request.question,
        &request.history,
        chrono::Local::now().date_naive(),
    )
    .await
    .map_err(|e| err("answer the question", e))
}

use log::{error, info};
use serde::Deserialize;
use tauri::{AppHandle, Runtime};

use super::answer::{ask_project, Answer, ConversationTurn};
use super::entities::{facts_by_type, facts_page, list_tickets, ticket_facts, tickets_page, FactRow, TicketSummary};
use super::history::{self, AskHistoryEntry, AskHistoryItem, Page};
use super::embeddings::{probe_ollama, OllamaProbe};
use super::indexer::{embedder_from_config, index_meeting_with_app, resolve_endpoint, IndexOutcome};
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
    let mut answer = ask_project(
        pool,
        embedder.as_ref(),
        &llm,
        &request.project_id,
        &request.question,
        &request.history,
        chrono::Local::now().date_naive(),
    )
    .await
    .map_err(|e| err("answer the question", e))?;
    // History is a convenience: a failed save must not lose the answer
    match history::save(pool, &request.project_id, &request.question, &answer).await {
        Ok(id) => answer.history_id = Some(id),
        Err(e) => error!("RAG: failed to save question to history: {}", e),
    }
    Ok(answer)
}

const MAX_PAGE_SIZE: i64 = 100;

fn page_bounds(limit: Option<i64>, offset: Option<i64>) -> (i64, i64) {
    (limit.unwrap_or(10).clamp(1, MAX_PAGE_SIZE), offset.unwrap_or(0).max(0))
}

/// Questions asked about a project, newest first.
#[tauri::command]
pub async fn rag_ask_history(
    state: tauri::State<'_, AppState>,
    project_id: String,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Page<AskHistoryEntry>, String> {
    let (limit, offset) = page_bounds(limit, offset);
    history::page(state.db_manager.pool(), &project_id, limit, offset)
        .await
        .map_err(|e| err("load question history", e))
}

#[tauri::command]
pub async fn rag_ask_history_item(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Option<AskHistoryItem>, String> {
    history::get(state.db_manager.pool(), &id)
        .await
        .map_err(|e| err("load saved answer", e))
}

#[tauri::command]
pub async fn rag_delete_ask_history(state: tauri::State<'_, AppState>, id: String) -> Result<bool, String> {
    history::delete(state.db_manager.pool(), &id)
        .await
        .map_err(|e| err("delete saved answer", e))
}

/// One page of a project's tickets, most recently discussed first.
#[tauri::command]
pub async fn rag_tickets_page(
    state: tauri::State<'_, AppState>,
    project_id: String,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Page<TicketSummary>, String> {
    let (limit, offset) = page_bounds(limit, offset);
    tickets_page(state.db_manager.pool(), &project_id, limit, offset)
        .await
        .map_err(|e| err("list tickets", e))
}

/// One page of a project's facts of one type (decision, action…), newest first.
#[tauri::command]
pub async fn rag_facts_page(
    state: tauri::State<'_, AppState>,
    project_id: String,
    fact_type: String,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Page<FactRow>, String> {
    let (limit, offset) = page_bounds(limit, offset);
    facts_page(state.db_manager.pool(), &project_id, &fact_type, limit, offset)
        .await
        .map_err(|e| err("list facts", e))
}

/// Tickets discussed in a project's meetings, with their latest recorded fact.
#[tauri::command]
pub async fn rag_list_tickets(
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> Result<Vec<TicketSummary>, String> {
    list_tickets(state.db_manager.pool(), &project_id)
        .await
        .map_err(|e| err("list tickets", e))
}

/// Timeline of facts recorded about one ticket, newest first.
#[tauri::command]
pub async fn rag_ticket_facts(
    state: tauri::State<'_, AppState>,
    entity_id: String,
) -> Result<Vec<FactRow>, String> {
    ticket_facts(state.db_manager.pool(), &entity_id)
        .await
        .map_err(|e| err("load ticket facts", e))
}

/// Recent decisions and action items of a project (optionally one type).
#[tauri::command]
pub async fn rag_list_facts(
    state: tauri::State<'_, AppState>,
    project_id: String,
    fact_type: String,
    limit: Option<usize>,
) -> Result<Vec<FactRow>, String> {
    facts_by_type(
        state.db_manager.pool(),
        &project_id,
        &[fact_type],
        None,
        None,
        None,
        limit.unwrap_or(50).clamp(1, 200),
    )
    .await
    .map_err(|e| err("list facts", e))
}

/// Checks an Ollama server for the embedding model. `endpoint` is what the user typed
/// (empty = same fallback as indexing: summary Ollama endpoint, then localhost).
#[tauri::command]
pub async fn rag_test_ollama(
    state: tauri::State<'_, AppState>,
    endpoint: Option<String>,
    model: String,
) -> Result<OllamaProbe, String> {
    let endpoint = resolve_endpoint(state.db_manager.pool(), endpoint.as_deref()).await;
    Ok(probe_ollama(endpoint.as_deref(), &model).await)
}

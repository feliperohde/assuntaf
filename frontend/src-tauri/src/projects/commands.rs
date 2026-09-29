use log::{error, info};
use tauri::{AppHandle, Runtime};

use crate::database::models::{Project, ProjectMember};
use crate::database::repositories::project::{
    ProjectInput, ProjectMemberInput, ProjectsRepository,
};
use crate::state::AppState;

fn db_error(action: &str, e: sqlx::Error) -> String {
    error!("Failed to {}: {}", action, e);
    format!("Failed to {}: {}", action, e)
}

#[tauri::command]
pub async fn list_projects(
    state: tauri::State<'_, AppState>,
    include_archived: Option<bool>,
) -> Result<Vec<Project>, String> {
    ProjectsRepository::list_projects(state.db_manager.pool(), include_archived.unwrap_or(false))
        .await
        .map_err(|e| db_error("list projects", e))
}

#[tauri::command]
pub async fn get_project(
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> Result<Option<Project>, String> {
    ProjectsRepository::get_project(state.db_manager.pool(), &project_id)
        .await
        .map_err(|e| db_error("get project", e))
}

#[tauri::command]
pub async fn create_project(
    state: tauri::State<'_, AppState>,
    project: ProjectInput,
) -> Result<Project, String> {
    info!("Creating project '{}'", project.name);
    ProjectsRepository::create_project(state.db_manager.pool(), &project)
        .await
        .map_err(|e| db_error("create project", e))
}

#[tauri::command]
pub async fn update_project(
    state: tauri::State<'_, AppState>,
    project_id: String,
    project: ProjectInput,
) -> Result<Project, String> {
    ProjectsRepository::update_project(state.db_manager.pool(), &project_id, &project)
        .await
        .map_err(|e| db_error("update project", e))
}

#[tauri::command]
pub async fn delete_project(
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> Result<bool, String> {
    info!("Deleting project {}", project_id);
    ProjectsRepository::delete_project(state.db_manager.pool(), &project_id)
        .await
        .map_err(|e| db_error("delete project", e))
}

#[tauri::command]
pub async fn set_meeting_project<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    project_id: String,
) -> Result<bool, String> {
    info!("Moving meeting {} to project {}", meeting_id, project_id);
    let moved = ProjectsRepository::set_meeting_project(state.db_manager.pool(), &meeting_id, &project_id)
        .await
        .map_err(|e| db_error("move meeting to project", e))?;
    if moved {
        // Rebuild facts against the new project's tickets and refresh the context header
        crate::rag::schedule_meeting_index(app, meeting_id);
    }
    Ok(moved)
}

#[tauri::command]
pub async fn list_project_members(
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> Result<Vec<ProjectMember>, String> {
    ProjectsRepository::list_members(state.db_manager.pool(), &project_id)
        .await
        .map_err(|e| db_error("list project members", e))
}

#[tauri::command]
pub async fn create_project_member(
    state: tauri::State<'_, AppState>,
    project_id: String,
    member: ProjectMemberInput,
) -> Result<ProjectMember, String> {
    ProjectsRepository::create_member(state.db_manager.pool(), &project_id, &member)
        .await
        .map_err(|e| db_error("create project member", e))
}

#[tauri::command]
pub async fn update_project_member(
    state: tauri::State<'_, AppState>,
    member_id: String,
    member: ProjectMemberInput,
) -> Result<ProjectMember, String> {
    ProjectsRepository::update_member(state.db_manager.pool(), &member_id, &member)
        .await
        .map_err(|e| db_error("update project member", e))
}

#[tauri::command]
pub async fn delete_project_member(
    state: tauri::State<'_, AppState>,
    member_id: String,
) -> Result<bool, String> {
    ProjectsRepository::delete_member(state.db_manager.pool(), &member_id)
        .await
        .map_err(|e| db_error("delete project member", e))
}

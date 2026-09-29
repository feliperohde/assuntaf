use crate::database::models::{Project, ProjectMember};
use chrono::Utc;
use serde::Deserialize;
use sqlx::{Connection, Error as SqlxError, SqlitePool};
use tracing::info;
use uuid::Uuid;

/// Project that owns meetings created before projects existed, and the fallback
/// when no project is specified.
pub const DEFAULT_PROJECT_ID: &str = "project-default";

const PROJECT_COLUMNS: &str = "id, name, description, context_md, glossary, ticket_patterns, color, archived, created_at, updated_at";
const MEMBER_COLUMNS: &str = "id, project_id, name, role, email, created_at, updated_at";

/// Editable project fields, shared by create and update.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInput {
    pub name: String,
    pub description: Option<String>,
    pub context_md: Option<String>,
    pub glossary: Option<String>,
    pub ticket_patterns: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub archived: bool,
}

/// Editable project member fields, shared by create and update.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMemberInput {
    pub name: String,
    pub role: Option<String>,
    pub email: Option<String>,
}

fn validate_name(name: &str) -> Result<(), SqlxError> {
    if name.trim().is_empty() {
        return Err(SqlxError::Protocol("name cannot be empty".to_string()));
    }
    Ok(())
}

pub struct ProjectsRepository;

impl ProjectsRepository {
    pub async fn list_projects(
        pool: &SqlitePool,
        include_archived: bool,
    ) -> Result<Vec<Project>, SqlxError> {
        let sql = if include_archived {
            format!("SELECT {PROJECT_COLUMNS} FROM projects ORDER BY (id = ?) DESC, name COLLATE NOCASE")
        } else {
            format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE archived = 0 ORDER BY (id = ?) DESC, name COLLATE NOCASE")
        };
        sqlx::query_as::<_, Project>(&sql)
            .bind(DEFAULT_PROJECT_ID)
            .fetch_all(pool)
            .await
    }

    pub async fn get_project(
        pool: &SqlitePool,
        project_id: &str,
    ) -> Result<Option<Project>, SqlxError> {
        sqlx::query_as::<_, Project>(&format!(
            "SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?"
        ))
        .bind(project_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn create_project(
        pool: &SqlitePool,
        input: &ProjectInput,
    ) -> Result<Project, SqlxError> {
        validate_name(&input.name)?;
        let id = format!("project-{}", Uuid::new_v4());
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO projects (id, name, description, context_md, glossary, ticket_patterns, color, archived, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(input.name.trim())
        .bind(&input.description)
        .bind(&input.context_md)
        .bind(&input.glossary)
        .bind(&input.ticket_patterns)
        .bind(&input.color)
        .bind(input.archived)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

        info!("Created project {} ({})", input.name.trim(), id);
        Self::get_project(pool, &id)
            .await?
            .ok_or(SqlxError::RowNotFound)
    }

    pub async fn update_project(
        pool: &SqlitePool,
        project_id: &str,
        input: &ProjectInput,
    ) -> Result<Project, SqlxError> {
        validate_name(&input.name)?;
        if project_id == DEFAULT_PROJECT_ID && input.archived {
            return Err(SqlxError::Protocol(
                "the default project cannot be archived".to_string(),
            ));
        }

        let result = sqlx::query(
            "UPDATE projects SET name = ?, description = ?, context_md = ?, glossary = ?, ticket_patterns = ?, color = ?, archived = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(input.name.trim())
        .bind(&input.description)
        .bind(&input.context_md)
        .bind(&input.glossary)
        .bind(&input.ticket_patterns)
        .bind(&input.color)
        .bind(input.archived)
        .bind(Utc::now())
        .bind(project_id)
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(SqlxError::RowNotFound);
        }
        Self::get_project(pool, project_id)
            .await?
            .ok_or(SqlxError::RowNotFound)
    }

    /// Deletes a project. Its meetings are moved to the default project rather than
    /// deleted, so no recording or transcript is lost.
    pub async fn delete_project(pool: &SqlitePool, project_id: &str) -> Result<bool, SqlxError> {
        if project_id == DEFAULT_PROJECT_ID {
            return Err(SqlxError::Protocol(
                "the default project cannot be deleted".to_string(),
            ));
        }

        let mut conn = pool.acquire().await?;
        let mut tx = conn.begin().await?;

        sqlx::query("UPDATE meetings SET project_id = ? WHERE project_id = ?")
            .bind(DEFAULT_PROJECT_ID)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;

        // Indexed chunks follow their meetings
        sqlx::query("UPDATE rag_chunks SET project_id = ? WHERE project_id = ?")
            .bind(DEFAULT_PROJECT_ID)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;

        // Extracted facts are tied to the project's tickets; they are rebuilt when
        // the moved meetings are reindexed.
        sqlx::query("DELETE FROM entity_facts WHERE project_id = ?")
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM entities WHERE project_id = ?")
            .bind(project_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM project_members WHERE project_id = ?")
            .bind(project_id)
            .execute(&mut *tx)
            .await?;

        let result = sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(project_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    /// Moves a meeting to another project.
    pub async fn set_meeting_project(
        pool: &SqlitePool,
        meeting_id: &str,
        project_id: &str,
    ) -> Result<bool, SqlxError> {
        if Self::get_project(pool, project_id).await?.is_none() {
            return Err(SqlxError::RowNotFound);
        }
        let mut conn = pool.acquire().await?;
        let mut tx = conn.begin().await?;
        let result = sqlx::query("UPDATE meetings SET project_id = ?, updated_at = ? WHERE id = ?")
            .bind(project_id)
            .bind(Utc::now())
            .bind(meeting_id)
            .execute(&mut *tx)
            .await?;
        // Indexed chunks follow their meeting so search stays scoped correctly
        sqlx::query("UPDATE rag_chunks SET project_id = ? WHERE meeting_id = ?")
            .bind(project_id)
            .bind(meeting_id)
            .execute(&mut *tx)
            .await?;
        // Facts reference the old project's tickets; drop them (re-extracted on reindex)
        let old_project: Option<(Option<String>,)> =
            sqlx::query_as("SELECT DISTINCT project_id FROM entity_facts WHERE meeting_id = ?")
                .bind(meeting_id)
                .fetch_optional(&mut *tx)
                .await?;
        sqlx::query("DELETE FROM entity_facts WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *tx)
            .await?;
        if let Some(old_project) = old_project.and_then(|(p,)| p) {
            crate::rag::entities::delete_orphan_entities(&mut *tx, &old_project).await?;
        }
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    /// Returns `project_id` when it names an existing project, otherwise the default project.
    pub async fn resolve_project_id(
        pool: &SqlitePool,
        project_id: Option<&str>,
    ) -> Result<String, SqlxError> {
        if let Some(id) = project_id.filter(|id| !id.trim().is_empty()) {
            if Self::get_project(pool, id).await?.is_some() {
                return Ok(id.to_string());
            }
        }
        Ok(DEFAULT_PROJECT_ID.to_string())
    }

    /// Builds a plain-text description of the project that owns `meeting_id` (name,
    /// description, context, glossary, members) for use as LLM background context.
    /// Returns `None` when the project has nothing beyond its name worth adding.
    pub async fn project_context_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<String>, SqlxError> {
        let project_id: Option<(Option<String>,)> =
            sqlx::query_as("SELECT project_id FROM meetings WHERE id = ?")
                .bind(meeting_id)
                .fetch_optional(pool)
                .await?;
        let Some(project_id) = project_id.and_then(|(id,)| id) else {
            return Ok(None);
        };
        Self::project_context(pool, &project_id).await
    }

    /// Plain-text description of a project for LLM background context.
    pub async fn project_context(
        pool: &SqlitePool,
        project_id: &str,
    ) -> Result<Option<String>, SqlxError> {
        let Some(project) = Self::get_project(pool, project_id).await? else {
            return Ok(None);
        };
        let members = Self::list_members(pool, project_id).await?;
        Ok(format_project_context(&project, &members))
    }

    pub async fn list_members(
        pool: &SqlitePool,
        project_id: &str,
    ) -> Result<Vec<ProjectMember>, SqlxError> {
        sqlx::query_as::<_, ProjectMember>(&format!(
            "SELECT {MEMBER_COLUMNS} FROM project_members WHERE project_id = ? ORDER BY name COLLATE NOCASE"
        ))
        .bind(project_id)
        .fetch_all(pool)
        .await
    }

    pub async fn create_member(
        pool: &SqlitePool,
        project_id: &str,
        input: &ProjectMemberInput,
    ) -> Result<ProjectMember, SqlxError> {
        validate_name(&input.name)?;
        if Self::get_project(pool, project_id).await?.is_none() {
            return Err(SqlxError::RowNotFound);
        }
        let id = format!("member-{}", Uuid::new_v4());
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO project_members (id, project_id, name, role, email, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(project_id)
        .bind(input.name.trim())
        .bind(&input.role)
        .bind(&input.email)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

        Self::get_member(pool, &id).await?.ok_or(SqlxError::RowNotFound)
    }

    pub async fn update_member(
        pool: &SqlitePool,
        member_id: &str,
        input: &ProjectMemberInput,
    ) -> Result<ProjectMember, SqlxError> {
        validate_name(&input.name)?;
        let result = sqlx::query(
            "UPDATE project_members SET name = ?, role = ?, email = ?, updated_at = ? WHERE id = ?",
        )
        .bind(input.name.trim())
        .bind(&input.role)
        .bind(&input.email)
        .bind(Utc::now())
        .bind(member_id)
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(SqlxError::RowNotFound);
        }
        Self::get_member(pool, member_id)
            .await?
            .ok_or(SqlxError::RowNotFound)
    }

    pub async fn delete_member(pool: &SqlitePool, member_id: &str) -> Result<bool, SqlxError> {
        let result = sqlx::query("DELETE FROM project_members WHERE id = ?")
            .bind(member_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn get_member(
        pool: &SqlitePool,
        member_id: &str,
    ) -> Result<Option<ProjectMember>, SqlxError> {
        sqlx::query_as::<_, ProjectMember>(&format!(
            "SELECT {MEMBER_COLUMNS} FROM project_members WHERE id = ?"
        ))
        .bind(member_id)
        .fetch_optional(pool)
        .await
    }
}

fn format_project_context(project: &Project, members: &[ProjectMember]) -> Option<String> {
    let non_empty = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);

    let mut sections = Vec::new();
    if let Some(description) = non_empty(&project.description) {
        sections.push(format!("Description: {description}"));
    }
    if let Some(context) = non_empty(&project.context_md) {
        sections.push(format!("Context:\n{context}"));
    }
    if let Some(glossary) = non_empty(&project.glossary) {
        sections.push(format!("Glossary:\n{glossary}"));
    }
    if !members.is_empty() {
        let list = members
            .iter()
            .map(|m| match m.role.as_deref().filter(|r| !r.trim().is_empty()) {
                Some(role) => format!("- {} ({})", m.name, role),
                None => format!("- {}", m.name),
            })
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("Members:\n{list}"));
    }

    if sections.is_empty() {
        return None;
    }
    Some(format!("Project: {}\n{}", project.name, sections.join("\n\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn input(name: &str) -> ProjectInput {
        ProjectInput {
            name: name.to_string(),
            description: None,
            context_md: Some("Contexto".to_string()),
            glossary: None,
            ticket_patterns: Some("ABC-\\d+".to_string()),
            color: None,
            archived: false,
        }
    }

    async fn insert_meeting(pool: &SqlitePool, id: &str, project_id: Option<&str>) {
        let now = Utc::now();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, project_id) VALUES (?, ?, ?, ?, ?)")
            .bind(id)
            .bind("Daily")
            .bind(now)
            .bind(now)
            .bind(project_id)
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn migration_creates_default_project() {
        let pool = test_pool().await;
        let projects = ProjectsRepository::list_projects(&pool, false).await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, DEFAULT_PROJECT_ID);
    }

    #[tokio::test]
    async fn create_update_and_list_projects() {
        let pool = test_pool().await;
        let project = ProjectsRepository::create_project(&pool, &input("Alpha"))
            .await
            .unwrap();
        assert_eq!(project.name, "Alpha");
        assert_eq!(project.ticket_patterns.as_deref(), Some("ABC-\\d+"));

        let mut changed = input("Alpha 2");
        changed.archived = true;
        let updated = ProjectsRepository::update_project(&pool, &project.id, &changed)
            .await
            .unwrap();
        assert_eq!(updated.name, "Alpha 2");
        assert!(updated.archived);

        assert_eq!(ProjectsRepository::list_projects(&pool, false).await.unwrap().len(), 1);
        assert_eq!(ProjectsRepository::list_projects(&pool, true).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn default_project_is_protected() {
        let pool = test_pool().await;
        assert!(ProjectsRepository::delete_project(&pool, DEFAULT_PROJECT_ID).await.is_err());
        let mut archived = input("Geral");
        archived.archived = true;
        assert!(
            ProjectsRepository::update_project(&pool, DEFAULT_PROJECT_ID, &archived)
                .await
                .is_err()
        );
        assert!(ProjectsRepository::create_project(&pool, &input("  ")).await.is_err());
    }

    #[tokio::test]
    async fn deleting_project_moves_meetings_to_default() {
        let pool = test_pool().await;
        let project = ProjectsRepository::create_project(&pool, &input("Beta"))
            .await
            .unwrap();
        insert_meeting(&pool, "meeting-1", Some(&project.id)).await;
        ProjectsRepository::create_member(
            &pool,
            &project.id,
            &ProjectMemberInput { name: "Ana".into(), role: None, email: None },
        )
        .await
        .unwrap();

        assert!(ProjectsRepository::delete_project(&pool, &project.id).await.unwrap());

        let (project_id,): (String,) =
            sqlx::query_as("SELECT project_id FROM meetings WHERE id = 'meeting-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(project_id, DEFAULT_PROJECT_ID);
        let (members,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM project_members")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(members, 0);
    }

    #[tokio::test]
    async fn move_meeting_and_resolve_project() {
        let pool = test_pool().await;
        let project = ProjectsRepository::create_project(&pool, &input("Gamma"))
            .await
            .unwrap();
        insert_meeting(&pool, "meeting-2", Some(DEFAULT_PROJECT_ID)).await;

        assert!(ProjectsRepository::set_meeting_project(&pool, "meeting-2", &project.id)
            .await
            .unwrap());
        assert!(ProjectsRepository::set_meeting_project(&pool, "meeting-2", "project-missing")
            .await
            .is_err());

        assert_eq!(
            ProjectsRepository::resolve_project_id(&pool, Some(&project.id)).await.unwrap(),
            project.id
        );
        assert_eq!(
            ProjectsRepository::resolve_project_id(&pool, Some("project-missing")).await.unwrap(),
            DEFAULT_PROJECT_ID
        );
        assert_eq!(
            ProjectsRepository::resolve_project_id(&pool, None).await.unwrap(),
            DEFAULT_PROJECT_ID
        );
    }

    #[tokio::test]
    async fn project_context_for_meeting_includes_details() {
        let pool = test_pool().await;
        let project = ProjectsRepository::create_project(&pool, &input("Delta"))
            .await
            .unwrap();
        ProjectsRepository::create_member(
            &pool,
            &project.id,
            &ProjectMemberInput { name: "Ana".into(), role: Some("PM".into()), email: None },
        )
        .await
        .unwrap();
        insert_meeting(&pool, "meeting-3", Some(&project.id)).await;
        insert_meeting(&pool, "meeting-4", Some(DEFAULT_PROJECT_ID)).await;

        let context = ProjectsRepository::project_context_for_meeting(&pool, "meeting-3")
            .await
            .unwrap()
            .unwrap();
        assert!(context.starts_with("Project: Delta"));
        assert!(context.contains("Contexto"));
        assert!(context.contains("- Ana (PM)"));

        // Default project has only a description
        let default_context = ProjectsRepository::project_context_for_meeting(&pool, "meeting-4")
            .await
            .unwrap()
            .unwrap();
        assert!(default_context.starts_with("Project: Geral"));
        assert!(ProjectsRepository::project_context_for_meeting(&pool, "missing")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn member_crud() {
        let pool = test_pool().await;
        let member = ProjectsRepository::create_member(
            &pool,
            DEFAULT_PROJECT_ID,
            &ProjectMemberInput { name: "Bruno".into(), role: Some("Dev".into()), email: None },
        )
        .await
        .unwrap();
        let updated = ProjectsRepository::update_member(
            &pool,
            &member.id,
            &ProjectMemberInput { name: "Bruno S.".into(), role: Some("Tech Lead".into()), email: None },
        )
        .await
        .unwrap();
        assert_eq!(updated.role.as_deref(), Some("Tech Lead"));
        assert_eq!(
            ProjectsRepository::list_members(&pool, DEFAULT_PROJECT_ID).await.unwrap().len(),
            1
        );
        assert!(ProjectsRepository::delete_member(&pool, &member.id).await.unwrap());
    }
}

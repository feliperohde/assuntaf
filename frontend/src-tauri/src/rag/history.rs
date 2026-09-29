//! History of questions asked on the Ask page, per project.

use chrono::Utc;
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use super::answer::Answer;

/// One page of a list plus the total number of rows, for pagination.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AskHistoryEntry {
    pub id: String,
    pub question: String,
    pub found: bool,
    pub citation_count: i64,
    pub created_at: String,
    pub all_projects: bool,
    /// Start of the answer, for list previews.
    pub answer_preview: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskHistoryItem {
    pub id: String,
    pub project_id: String,
    pub question: String,
    pub created_at: String,
    pub all_projects: bool,
    /// The answer exactly as it was returned (same shape as `Answer`).
    pub answer: serde_json::Value,
}

const PREVIEW_CHARS: usize = 160;

fn preview(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= PREVIEW_CHARS {
        flat
    } else {
        format!("{}…", flat.chars().take(PREVIEW_CHARS).collect::<String>())
    }
}

pub async fn save(
    pool: &SqlitePool,
    project_id: &str,
    question: &str,
    answer: &Answer,
    all_projects: bool,
) -> Result<String, sqlx::Error> {
    let id = format!("ask-{}", Uuid::new_v4());
    let json = serde_json::to_string(answer).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
    sqlx::query(
        "INSERT INTO ask_history (id, project_id, question, answer_json, found, citation_count, created_at, all_projects)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(project_id)
    .bind(question.trim())
    .bind(json)
    .bind(answer.found)
    .bind(answer.citations.len() as i64)
    .bind(Utc::now().to_rfc3339())
    .bind(all_projects)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn page(pool: &SqlitePool, project_id: &str, limit: i64, offset: i64) -> Result<Page<AskHistoryEntry>, sqlx::Error> {
    let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ask_history WHERE project_id = ?")
        .bind(project_id)
        .fetch_one(pool)
        .await?;
    let rows: Vec<(String, String, bool, i64, String, String, bool)> = sqlx::query_as(
        "SELECT id, question, found, citation_count, created_at, answer_json, all_projects FROM ask_history
         WHERE project_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(project_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    let items = rows
        .into_iter()
        .map(|(id, question, found, citation_count, created_at, json, all_projects)| {
            let answer = serde_json::from_str::<serde_json::Value>(&json)
                .ok()
                .and_then(|v| v.get("answer").and_then(|a| a.as_str()).map(preview))
                .unwrap_or_default();
            AskHistoryEntry { id, question, found, citation_count, created_at, all_projects, answer_preview: answer }
        })
        .collect();
    Ok(Page { items, total })
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<AskHistoryItem>, sqlx::Error> {
    let row: Option<(String, String, String, String, String, bool)> = sqlx::query_as(
        "SELECT id, project_id, question, created_at, answer_json, all_projects FROM ask_history WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id, project_id, question, created_at, json, all_projects)| AskHistoryItem {
        id,
        project_id,
        question,
        created_at,
        all_projects,
        answer: serde_json::from_str(&json).unwrap_or(serde_json::Value::Null),
    }))
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM ask_history WHERE id = ?").bind(id).execute(pool).await?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rag::answer::QueryPlan;
    use sqlx::sqlite::SqlitePoolOptions;

    fn answer(text: &str) -> Answer {
        Answer {
            answer: text.to_string(),
            found: true,
            citations: Vec::new(),
            plan: QueryPlan { search_query: "q".into(), ..Default::default() },
            filters_relaxed: false,
            notice: None,
            history_id: None,
        }
    }

    #[tokio::test]
    async fn saves_pages_reopens_and_deletes() {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name, created_at, updated_at) VALUES ('p1', 'P', 'x', 'x')")
            .execute(&pool).await.unwrap();

        let mut ids = Vec::new();
        for i in 0..12 {
            ids.push(save(&pool, "p1", &format!("Pergunta {i}"), &answer(&"resposta ".repeat(50)), i % 2 == 0).await.unwrap());
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        let first = page(&pool, "p1", 10, 0).await.unwrap();
        assert_eq!(first.total, 12);
        assert_eq!(first.items.len(), 10);
        assert_eq!(first.items[0].question, "Pergunta 11"); // newest first
        assert!(first.items[0].answer_preview.ends_with('…'));
        assert_eq!(page(&pool, "p1", 10, 10).await.unwrap().items.len(), 2);
        assert_eq!(page(&pool, "other", 10, 0).await.unwrap().total, 0);

        let item = get(&pool, &ids[3]).await.unwrap().unwrap();
        assert_eq!(item.question, "Pergunta 3");
        assert_eq!(item.answer["found"], true);
        assert!(!item.all_projects); // i = 3 was saved scoped
        assert!(get(&pool, &ids[4]).await.unwrap().unwrap().all_projects);

        assert!(delete(&pool, &ids[3]).await.unwrap());
        assert!(get(&pool, &ids[3]).await.unwrap().is_none());
    }
}

//! Structured facts extracted from meetings (phase 4 of docs/plans/projects-and-rag.md).
//!
//! After a meeting is indexed, the LLM reads its passages and lists statements
//! about tickets (status, blockers) plus decisions and action items. Each fact
//! keeps the passage it came from, so "why is ABC-123 blocked?" is answered from
//! the latest dated fact about that ticket, with a citation.

use anyhow::{anyhow, Result};
use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::{Connection, FromRow, SqlitePool};
use uuid::Uuid;

use super::llm::ChatModel;

/// Used when a project defines no ticket patterns: uppercase key, dash, number (e.g. PAY-42).
pub const DEFAULT_TICKET_PATTERN: &str = r"\b[A-Z][A-Z0-9]{1,9}-\d{1,6}\b";
pub const FACT_TYPES: &[&str] = &["status", "blocker", "decision", "action"];
/// Passages sent per extraction call.
const EXTRACTION_BATCH_CHARS: usize = 12_000;

/// Recognizes ticket IDs using a project's patterns (comma or newline separated
/// regexes, matched case-insensitively) or the default pattern.
pub struct TicketMatcher {
    regexes: Vec<Regex>,
}

impl TicketMatcher {
    pub fn new(patterns: Option<&str>) -> Self {
        let regexes: Vec<Regex> = patterns
            .unwrap_or_default()
            .split(|c| c == ',' || c == '\n')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .filter_map(|p| match Regex::new(&format!(r"(?i)\b(?:{p})\b")) {
                Ok(re) => Some(re),
                Err(e) => {
                    log::warn!("RAG: ignoring invalid ticket pattern '{}': {}", p, e);
                    None
                }
            })
            .collect();
        if regexes.is_empty() {
            Self { regexes: vec![Regex::new(DEFAULT_TICKET_PATTERN).expect("valid default pattern")] }
        } else {
            Self { regexes }
        }
    }

    /// All ticket IDs in `text`, uppercased, in first-seen order.
    pub fn find_all(&self, text: &str) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for re in &self.regexes {
            for m in re.find_iter(text) {
                let key = m.as_str().to_uppercase();
                if !found.contains(&key) {
                    found.push(key);
                }
            }
        }
        found
    }

    /// Returns the normalized ID when `candidate` is exactly one ticket ID.
    /// IDs are canonically uppercase, so the uppercased form is accepted too
    /// (the LLM may echo "abc-123").
    pub fn normalize(&self, candidate: &str) -> Option<String> {
        let candidate = candidate.trim();
        let upper = candidate.to_uppercase();
        let exact = |c: &str| -> Option<String> {
            self.regexes
                .iter()
                .filter_map(|re| re.find(c))
                .find(|m| m.start() == 0 && m.end() == c.len())
                .map(|m| m.as_str().to_uppercase())
        };
        exact(candidate).or_else(|| exact(&upper))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedFact {
    pub fact_type: String,
    pub ticket: Option<String>,
    pub content: String,
    pub owner: Option<String>,
    /// Index into the passages given to the extractor.
    pub source: usize,
}

/// Parses the extractor's JSON, keeping only well-formed facts.
pub fn parse_facts(reply: &str, passage_count: usize, matcher: &TicketMatcher) -> Vec<ExtractedFact> {
    #[derive(Deserialize)]
    struct RawFact {
        #[serde(rename = "type")]
        fact_type: Option<String>,
        ticket: Option<String>,
        content: Option<String>,
        owner: Option<String>,
        source: Option<serde_json::Value>,
    }
    #[derive(Deserialize)]
    struct RawReply {
        facts: Vec<RawFact>,
    }

    let raw: Vec<RawFact> = {
        let object = reply.find('{').zip(reply.rfind('}')).filter(|(s, e)| s < e);
        let array = reply.find('[').zip(reply.rfind(']')).filter(|(s, e)| s < e);
        let from_object = object.and_then(|(s, e)| serde_json::from_str::<RawReply>(&reply[s..=e]).ok());
        match from_object {
            Some(parsed) => parsed.facts,
            None => array
                .and_then(|(s, e)| serde_json::from_str::<Vec<RawFact>>(&reply[s..=e]).ok())
                .unwrap_or_default(),
        }
    };

    raw.into_iter()
        .filter_map(|f| {
            let fact_type = f.fact_type?.trim().to_lowercase();
            if !FACT_TYPES.contains(&fact_type.as_str()) {
                return None;
            }
            let content = f.content?.trim().to_string();
            if content.is_empty() {
                return None;
            }
            // Sources are 1-based in the prompt; accept numbers or numeric strings
            let source = match f.source? {
                serde_json::Value::Number(n) => n.as_u64()? as usize,
                serde_json::Value::String(s) => s.trim().trim_matches(|c| c == '[' || c == ']').parse().ok()?,
                _ => return None,
            };
            if source == 0 || source > passage_count {
                return None;
            }
            Some(ExtractedFact {
                fact_type,
                ticket: f.ticket.as_deref().and_then(|t| matcher.normalize(t)),
                content,
                owner: f.owner.map(|o| o.trim().to_string()).filter(|o| !o.is_empty()),
                source: source - 1,
            })
        })
        .collect()
}

const EXTRACTION_SYSTEM: &str = "You extract structured facts from excerpts of a software team's meeting. \
Reply with ONLY JSON: {\"facts\": [{\"type\": \"status\" | \"blocker\" | \"decision\" | \"action\", \"ticket\": string or null, \"content\": string, \"owner\": string or null, \"source\": number}]}.\n\
- status: the current state of a ticket or task (e.g. in progress, in review, done).\n\
- blocker: something blocked or at risk, including why.\n\
- decision: something the team decided.\n\
- action: a task someone committed to do; owner is who will do it.\n\
- ticket: a ticket/issue ID exactly as said, only when the fact is about that ticket; otherwise null.\n\
- content: one self-contained sentence in the language of the meeting, including the reason when one is given.\n\
- source: the number of the excerpt the fact comes from.\n\
Only include facts that are actually stated. If there are none, reply {\"facts\": []}.";

#[derive(Debug, Clone, FromRow)]
struct Passage {
    id: String,
    text: String,
    start_time: Option<f64>,
}

fn format_time(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

/// Extracts and stores the facts of one meeting, replacing previous ones.
/// Returns how many facts were stored.
pub async fn extract_meeting_facts(pool: &SqlitePool, llm: &dyn ChatModel, meeting_id: &str) -> Result<usize> {
    let meeting: Option<(Option<String>, String, String)> = sqlx::query_as(
        "SELECT project_id, title, CAST(created_at AS TEXT) FROM meetings WHERE id = ?",
    )
    .bind(meeting_id)
    .fetch_optional(pool)
    .await?;
    let (project_id, title, meeting_date) = meeting.ok_or_else(|| anyhow!("Meeting {} not found", meeting_id))?;
    let project_id = project_id.ok_or_else(|| anyhow!("Meeting {} has no project", meeting_id))?;

    let patterns: Option<(Option<String>,)> = sqlx::query_as("SELECT ticket_patterns FROM projects WHERE id = ?")
        .bind(&project_id)
        .fetch_optional(pool)
        .await?;
    let matcher = TicketMatcher::new(patterns.and_then(|(p,)| p).as_deref());

    let passages: Vec<Passage> = sqlx::query_as(
        "SELECT id, text, start_time FROM rag_chunks WHERE meeting_id = ? ORDER BY chunk_index",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await?;

    // Batch passages so each call stays within a modest context window
    let mut facts: Vec<(ExtractedFact, &Passage)> = Vec::new();
    let mut start = 0;
    while start < passages.len() {
        let mut end = start;
        let mut chars = 0;
        while end < passages.len() && (end == start || chars + passages[end].text.len() <= EXTRACTION_BATCH_CHARS) {
            chars += passages[end].text.len();
            end += 1;
        }
        let batch = &passages[start..end];
        let excerpts = batch
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let time = p.start_time.map(|t| format!(" ({})", format_time(t))).unwrap_or_default();
                format!("[{}]{}\n{}", i + 1, time, p.text)
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let known = matcher.find_all(&batch.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n"));
        let prompt = format!(
            "Meeting: \"{title}\" ({date})\nTicket IDs mentioned: {known}\n\n<excerpts>\n{excerpts}\n</excerpts>",
            date = meeting_date.get(..10).unwrap_or(&meeting_date),
            known = if known.is_empty() { "none".to_string() } else { known.join(", ") },
        );
        let reply = llm.complete(EXTRACTION_SYSTEM, &prompt).await?;
        for fact in parse_facts(&reply, batch.len(), &matcher) {
            let passage = &batch[fact.source];
            facts.push((fact, passage));
        }
        start = end;
    }

    // Drop duplicates (overlapping passages often restate the same fact)
    let mut seen = std::collections::HashSet::new();
    facts.retain(|(f, _)| seen.insert((f.fact_type.clone(), f.ticket.clone(), f.content.to_lowercase())));

    let mut conn = pool.acquire().await?;
    let mut tx = conn.begin().await?;
    sqlx::query("DELETE FROM entity_facts WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *tx)
        .await?;
    let now = Utc::now();
    for (fact, passage) in &facts {
        let entity_id = match &fact.ticket {
            Some(key) => {
                sqlx::query(
                    "INSERT INTO entities (id, project_id, entity_type, key, display_name, source, created_at, updated_at)
                     VALUES (?, ?, 'ticket', ?, ?, 'meeting', ?, ?)
                     ON CONFLICT(project_id, entity_type, key) DO UPDATE SET updated_at = excluded.updated_at",
                )
                .bind(format!("entity-{}", Uuid::new_v4()))
                .bind(&project_id)
                .bind(key)
                .bind(key)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                let (id,): (String,) = sqlx::query_as(
                    "SELECT id FROM entities WHERE project_id = ? AND entity_type = 'ticket' AND key = ?",
                )
                .bind(&project_id)
                .bind(key)
                .fetch_one(&mut *tx)
                .await?;
                Some(id)
            }
            None => None,
        };
        sqlx::query(
            "INSERT INTO entity_facts (id, entity_id, project_id, meeting_id, chunk_id, fact_type, content, owner, start_time, meeting_date, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("fact-{}", Uuid::new_v4()))
        .bind(entity_id)
        .bind(&project_id)
        .bind(meeting_id)
        .bind(&passage.id)
        .bind(&fact.fact_type)
        .bind(&fact.content)
        .bind(&fact.owner)
        .bind(passage.start_time)
        .bind(&meeting_date)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    delete_orphan_entities(&mut tx, &project_id).await?;
    tx.commit().await?;

    log::info!("RAG: extracted {} facts from meeting {}", facts.len(), meeting_id);
    Ok(facts.len())
}

/// Removes ticket entities that no longer have any fact.
pub async fn delete_orphan_entities(conn: &mut sqlx::SqliteConnection, project_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM entities WHERE project_id = ? AND source = 'meeting'
         AND NOT EXISTS (SELECT 1 FROM entity_facts f WHERE f.entity_id = entities.id)",
    )
    .bind(project_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct FactRow {
    pub id: String,
    pub ticket: Option<String>,
    pub fact_type: String,
    pub content: String,
    pub owner: Option<String>,
    pub meeting_id: String,
    pub meeting_title: String,
    pub meeting_date: String,
    pub start_time: Option<f64>,
    pub chunk_id: Option<String>,
}

const FACT_SELECT: &str = "SELECT f.id, e.key AS ticket, f.fact_type, f.content, f.owner, f.meeting_id,
        m.title AS meeting_title, f.meeting_date, f.start_time, f.chunk_id
     FROM entity_facts f
     JOIN meetings m ON m.id = f.meeting_id
     LEFT JOIN entities e ON e.id = f.entity_id";

/// Facts about the given tickets, newest meeting first.
pub async fn facts_for_tickets(
    pool: &SqlitePool,
    project_id: &str,
    tickets: &[String],
    limit: usize,
) -> Result<Vec<FactRow>, sqlx::Error> {
    let mut rows = Vec::new();
    for ticket in tickets {
        let mut found: Vec<FactRow> = sqlx::query_as(&format!(
            "{FACT_SELECT} WHERE f.project_id = ? AND e.key = ? ORDER BY f.meeting_date DESC, f.start_time DESC LIMIT ?"
        ))
        .bind(project_id)
        .bind(ticket)
        .bind(limit as i64)
        .fetch_all(pool)
        .await?;
        rows.append(&mut found);
    }
    rows.sort_by(|a, b| b.meeting_date.cmp(&a.meeting_date));
    rows.truncate(limit);
    Ok(rows)
}

/// Facts of the given types (e.g. decisions), newest first, within optional date/meeting bounds.
pub async fn facts_by_type(
    pool: &SqlitePool,
    project_id: &str,
    fact_types: &[String],
    meeting_id: Option<&str>,
    date_from: Option<&str>,
    date_to: Option<&str>,
    limit: usize,
) -> Result<Vec<FactRow>, sqlx::Error> {
    let mut rows = Vec::new();
    for fact_type in fact_types {
        let mut found: Vec<FactRow> = sqlx::query_as(&format!(
            "{FACT_SELECT} WHERE f.project_id = ? AND f.fact_type = ?
               AND (? IS NULL OR f.meeting_id = ?)
               AND (? IS NULL OR f.meeting_date >= ?)
               AND (? IS NULL OR f.meeting_date < ?)
             ORDER BY f.meeting_date DESC LIMIT ?"
        ))
        .bind(project_id)
        .bind(fact_type)
        .bind(meeting_id)
        .bind(meeting_id)
        .bind(date_from)
        .bind(date_from)
        .bind(date_to)
        .bind(date_to)
        .bind(limit as i64)
        .fetch_all(pool)
        .await?;
        rows.append(&mut found);
    }
    rows.sort_by(|a, b| b.meeting_date.cmp(&a.meeting_date));
    rows.truncate(limit);
    Ok(rows)
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TicketSummary {
    pub entity_id: String,
    pub key: String,
    pub fact_count: i64,
    pub last_meeting_date: String,
    pub latest_fact_type: String,
    pub latest_content: String,
}

/// Tickets of a project with their most recent fact, most recently discussed first.
pub async fn list_tickets(pool: &SqlitePool, project_id: &str) -> Result<Vec<TicketSummary>, sqlx::Error> {
    sqlx::query_as(
        "SELECT e.id AS entity_id, e.key,
                (SELECT COUNT(*) FROM entity_facts f WHERE f.entity_id = e.id) AS fact_count,
                l.meeting_date AS last_meeting_date, l.fact_type AS latest_fact_type, l.content AS latest_content
         FROM entities e
         JOIN entity_facts l ON l.id = (
             SELECT f.id FROM entity_facts f WHERE f.entity_id = e.id
             ORDER BY f.meeting_date DESC, f.start_time DESC LIMIT 1)
         WHERE e.project_id = ? AND e.entity_type = 'ticket'
         ORDER BY l.meeting_date DESC, e.key",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
}

/// All facts about one ticket, newest first.
pub async fn ticket_facts(pool: &SqlitePool, entity_id: &str) -> Result<Vec<FactRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "{FACT_SELECT} WHERE f.entity_id = ? ORDER BY f.meeting_date DESC, f.start_time DESC"
    ))
    .bind(entity_id)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rag::indexer::index_meeting;
    use crate::rag::embeddings::EmbeddingProvider;
    use async_trait::async_trait;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Mutex;

    #[test]
    fn default_matcher_finds_uppercase_ids() {
        let m = TicketMatcher::new(None);
        assert_eq!(m.find_all("ABC-123 e PAY-9, de novo ABC-123; covid-19 não"), vec!["ABC-123", "PAY-9"]);
        assert_eq!(m.normalize(" ABC-123 "), Some("ABC-123".into()));
        assert_eq!(m.normalize("ABC-123 foo"), None);
    }

    #[test]
    fn project_patterns_are_case_insensitive_and_invalid_ones_skipped() {
        let m = TicketMatcher::new(Some("PAY-\\d+, (bad\nOPS-\\d+"));
        assert_eq!(m.find_all("o pay-42 e o OPS-7 e ABC-1"), vec!["PAY-42", "OPS-7"]);
        assert_eq!(m.normalize("pay-42"), Some("PAY-42".into()));
        assert_eq!(m.normalize("ABC-1"), None);
    }

    #[test]
    fn parse_facts_validates_each_field() {
        let m = TicketMatcher::new(None);
        let reply = r#"Here: {"facts": [
            {"type": "Blocker", "ticket": "ABC-123", "content": "Bloqueado por falta de acesso", "owner": null, "source": 2},
            {"type": "action", "ticket": null, "content": "Ana abre o chamado", "owner": "Ana", "source": "1"},
            {"type": "rumor", "content": "x", "source": 1},
            {"type": "decision", "content": " ", "source": 1},
            {"type": "decision", "content": "Adiar release", "source": 9},
            {"type": "status", "ticket": "not a ticket", "content": "Em revisão", "source": 1}
        ]}"#;
        let facts = parse_facts(reply, 2, &m);
        assert_eq!(facts.len(), 3);
        assert_eq!(facts[0], ExtractedFact {
            fact_type: "blocker".into(),
            ticket: Some("ABC-123".into()),
            content: "Bloqueado por falta de acesso".into(),
            owner: None,
            source: 1,
        });
        assert_eq!(facts[1].owner.as_deref(), Some("Ana"));
        assert_eq!(facts[2].ticket, None);

        assert!(parse_facts("nothing", 2, &m).is_empty());
        assert_eq!(parse_facts(r#"[{"type":"decision","content":"Sim","source":1}]"#, 1, &m).len(), 1);
    }

    struct NullEmbedder;
    #[async_trait]
    impl EmbeddingProvider for NullEmbedder {
        fn model_id(&self) -> &str {
            "null"
        }
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![1.0]).collect())
        }
    }

    struct FixedLlm(Mutex<Vec<String>>);
    #[async_trait]
    impl ChatModel for FixedLlm {
        async fn complete(&self, _system: &str, _user: &str) -> Result<String> {
            Ok(self.0.lock().unwrap().remove(0))
        }
    }

    async fn setup() -> SqlitePool {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name, created_at, updated_at) VALUES ('p1', 'P', 'x', 'x')")
            .execute(&pool)
            .await
            .unwrap();
        for (id, date, text) in [
            ("m1", "2026-09-20 10:00:00+00:00", "ABC-123 está bloqueado porque falta acesso ao ambiente"),
            ("m2", "2026-09-27 10:00:00+00:00", "ABC-123 foi desbloqueado e está em revisão"),
        ] {
            sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, project_id) VALUES (?, 'Daily', ?, ?, 'p1')")
                .bind(id)
                .bind(date)
                .bind(date)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time) VALUES (?, ?, ?, '0', 60.0, 70.0)")
                .bind(format!("{id}-t"))
                .bind(id)
                .bind(text)
                .execute(&pool)
                .await
                .unwrap();
            index_meeting(&pool, &NullEmbedder, id).await.unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn extracts_stores_and_queries_ticket_facts() {
        let pool = setup().await;
        let llm = FixedLlm(Mutex::new(vec![
            r#"{"facts":[{"type":"blocker","ticket":"ABC-123","content":"Bloqueado: falta acesso ao ambiente","source":1},
                        {"type":"blocker","ticket":"ABC-123","content":"Bloqueado: falta acesso ao ambiente","source":1}]}"#.into(),
            r#"{"facts":[{"type":"status","ticket":"abc-123","content":"Desbloqueado, em revisão","source":1},
                        {"type":"decision","ticket":null,"content":"Release na sexta","source":1}]}"#.into(),
        ]));
        assert_eq!(extract_meeting_facts(&pool, &llm, "m1").await.unwrap(), 1); // duplicate dropped
        assert_eq!(extract_meeting_facts(&pool, &llm, "m2").await.unwrap(), 2);

        let facts = facts_for_tickets(&pool, "p1", &["ABC-123".into()], 10).await.unwrap();
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].meeting_id, "m2"); // newest first
        assert_eq!(facts[0].start_time, Some(60.0));
        assert!(facts[0].chunk_id.is_some());

        let tickets = list_tickets(&pool, "p1").await.unwrap();
        assert_eq!(tickets.len(), 1);
        assert_eq!(tickets[0].fact_count, 2);
        assert_eq!(tickets[0].latest_fact_type, "status");
        assert_eq!(ticket_facts(&pool, &tickets[0].entity_id).await.unwrap().len(), 2);

        let decisions = facts_by_type(&pool, "p1", &["decision".into()], None, Some("2026-09-25"), None, 10)
            .await
            .unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].ticket, None);
    }

    #[tokio::test]
    async fn reextraction_replaces_facts_and_removes_orphan_tickets() {
        let pool = setup().await;
        let llm = FixedLlm(Mutex::new(vec![
            r#"{"facts":[{"type":"blocker","ticket":"ABC-123","content":"Bloqueado","source":1}]}"#.into(),
            r#"{"facts":[]}"#.into(),
        ]));
        extract_meeting_facts(&pool, &llm, "m1").await.unwrap();
        assert_eq!(list_tickets(&pool, "p1").await.unwrap().len(), 1);
        extract_meeting_facts(&pool, &llm, "m1").await.unwrap();
        assert!(list_tickets(&pool, "p1").await.unwrap().is_empty());
        let (entities,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM entities").fetch_one(&pool).await.unwrap();
        assert_eq!(entities, 0);
    }
}

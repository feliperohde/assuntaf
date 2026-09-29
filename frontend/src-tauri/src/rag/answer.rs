//! "Ask the project": answers questions about a project's meetings with citations.
//!
//! Flow (docs/plans/projects-and-rag.md, phase 5):
//! 1. Planner — one LLM call turns the question into a search query plus optional
//!    filters (date range, a specific meeting), resolving "last Tuesday" against today.
//! 2. Retrieval — hybrid search with those filters; if filters leave nothing, retry
//!    unfiltered (a planner mistake must not hide evidence).
//! 3. Evidence gate — no passages means "not found", without asking the LLM to guess.
//! 4. Answer — the LLM answers only from numbered excerpts and cites them as [n].

use std::collections::HashMap;
use anyhow::Result;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::embeddings::EmbeddingProvider;
use super::entities::{facts_by_type, facts_for_tickets, FactRow, TicketMatcher, FACT_TYPES};
use super::llm::ChatModel;
use super::qdrant::QdrantStore;
use super::retriever::{hybrid_search_with, SearchResult};
use super::store::SearchFilters;
use crate::database::repositories::project::ProjectsRepository;

const PASSAGES_FOR_ANSWER: usize = 8;
const FACTS_FOR_ANSWER: usize = 6;
const MEETINGS_IN_PLANNER: i64 = 60;
const HISTORY_TURNS: usize = 4;
const HISTORY_ANSWER_CHARS: usize = 600;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTurn {
    pub question: String,
    pub answer: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    /// The [n] number used in the answer.
    pub index: usize,
    pub chunk_id: String,
    pub meeting_id: String,
    pub meeting_title: String,
    pub meeting_date: String,
    pub kind: String,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub excerpt: String,
    /// Set when searching all projects.
    pub project_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueryPlan {
    pub search_query: String,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub meeting_id: Option<String>,
    /// Ticket IDs the question is about.
    pub tickets: Vec<String>,
    /// Kinds of extracted facts that answer the question (decision, action, blocker, status).
    pub fact_types: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub answer: String,
    /// False when no relevant passage was found (the answer is then empty).
    pub found: bool,
    pub citations: Vec<Citation>,
    pub plan: QueryPlan,
    /// True when the planner's filters returned nothing and the search was retried without them.
    pub filters_relaxed: bool,
    /// Set when semantic search was unavailable (keyword results only).
    pub notice: Option<String>,
    /// Id in the Ask history, once saved.
    pub history_id: Option<String>,
}

#[derive(Debug, Clone)]
struct MeetingInfo {
    id: String,
    title: String,
    date: String,
}

/// Project name of each evidence's meeting.
async fn meeting_project_names(pool: &SqlitePool, evidence: &[Evidence]) -> Result<HashMap<String, String>> {
    let mut names = HashMap::new();
    for e in evidence {
        if names.contains_key(&e.meeting_id) {
            continue;
        }
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT p.name FROM meetings m JOIN projects p ON p.id = m.project_id WHERE m.id = ?",
        )
        .bind(&e.meeting_id)
        .fetch_optional(pool)
        .await?;
        if let Some((name,)) = row {
            names.insert(e.meeting_id.clone(), name);
        }
    }
    Ok(names)
}

async fn recent_meetings(pool: &SqlitePool, project_id: Option<&str>) -> Result<Vec<MeetingInfo>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, title, CAST(created_at AS TEXT) FROM meetings WHERE (? IS NULL OR project_id = ?)
         ORDER BY created_at DESC LIMIT ?",
    )
    .bind(project_id)
    .bind(project_id)
    .bind(MEETINGS_IN_PLANNER)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, title, date)| MeetingInfo { id, title, date: date.get(..10).unwrap_or(&date).to_string() })
        .collect())
}

fn format_history(history: &[ConversationTurn]) -> String {
    let start = history.len().saturating_sub(HISTORY_TURNS);
    history[start..]
        .iter()
        .map(|turn| {
            // Old [n] markers point at previous excerpts; drop them so they can't be
            // confused with the numbering of the current ones.
            let answer: String = strip_citations(&turn.answer).chars().take(HISTORY_ANSWER_CHARS).collect();
            format!("Q: {}\nA: {}", turn.question, answer)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

const PLANNER_SYSTEM: &str = "You convert a user's question about their team's meetings into a search plan. \
Reply with ONLY a JSON object, no prose: \
{\"search_query\": string, \"date_from\": \"YYYY-MM-DD\" or null, \"date_to\": \"YYYY-MM-DD\" or null, \"meeting_id\": string or null, \"tickets\": [string], \"fact_types\": [string]}. \
search_query: the key terms to search for (keep names, ticket IDs and technical terms verbatim; resolve pronouns using the conversation). \
date_from is inclusive and date_to is exclusive; set them only when the question refers to a time (e.g. \"yesterday\", \"last week\", \"on the 22nd\"). \
meeting_id: set only when the question clearly refers to one specific meeting from the list; copy its id exactly. \
tickets: ticket/issue IDs the question is about (e.g. [\"ABC-123\"]), else []. \
fact_types: which recorded facts would answer it: \"decision\" (what was decided), \"action\" (tasks, who will do what), \"blocker\" (what is blocked and why), \"status\" (state of a ticket); [] if none apply.";

fn planner_prompt(question: &str, today: NaiveDate, meetings: &[MeetingInfo], history: &[ConversationTurn]) -> String {
    let meeting_list = meetings
        .iter()
        .map(|m| format!("{} | {} | {}", m.id, m.date, m.title))
        .collect::<Vec<_>>()
        .join("\n");
    let history = format_history(history);
    format!(
        "Today is {today} ({weekday}).\n\nMeetings (id | date | title):\n{meeting_list}\n\n{history_block}Question: {question}",
        weekday = today.format("%A"),
        history_block = if history.is_empty() { String::new() } else { format!("Conversation so far:\n{history}\n\n") },
    )
}

/// Parses the planner's reply, discarding anything invalid. Never fails: the
/// fallback is the raw question with no filters.
pub fn parse_plan(reply: &str, question: &str, known_meeting_ids: &[String], matcher: &TicketMatcher) -> QueryPlan {
    #[derive(Deserialize)]
    struct RawPlan {
        search_query: Option<String>,
        date_from: Option<String>,
        date_to: Option<String>,
        meeting_id: Option<String>,
        #[serde(default)]
        tickets: Vec<String>,
        #[serde(default)]
        fact_types: Vec<String>,
    }

    let fallback = QueryPlan { search_query: question.to_string(), ..Default::default() };
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else {
        return fallback;
    };
    if end <= start {
        return fallback;
    }
    let Ok(raw) = serde_json::from_str::<RawPlan>(&reply[start..=end]) else {
        return fallback;
    };

    let valid_date = |d: Option<String>| {
        d.map(|d| d.trim().to_string())
            .filter(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok())
    };
    let mut date_from = valid_date(raw.date_from);
    let mut date_to = valid_date(raw.date_to);
    if let (Some(from), Some(to)) = (&date_from, &date_to) {
        if from >= to {
            // Treat an inverted/empty range as unusable rather than guessing
            date_from = None;
            date_to = None;
        }
    }

    QueryPlan {
        search_query: raw
            .search_query
            .map(|q| q.trim().to_string())
            .filter(|q| !q.is_empty())
            .unwrap_or_else(|| question.to_string()),
        date_from,
        date_to,
        meeting_id: raw.meeting_id.filter(|id| known_meeting_ids.contains(id)),
        tickets: dedup(raw.tickets.iter().filter_map(|t| matcher.normalize(t))),
        fact_types: dedup(
            raw.fact_types
                .iter()
                .map(|t| t.trim().to_lowercase())
                .filter(|t| FACT_TYPES.contains(&t.as_str())),
        ),
    }
}

fn dedup(items: impl Iterator<Item = String>) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

/// A passage or recorded fact shown to the answer model as a numbered excerpt.
struct Evidence {
    id: String,
    meeting_id: String,
    meeting_title: String,
    meeting_date: String,
    kind: String,
    start_time: Option<f64>,
    end_time: Option<f64>,
    text: String,
}

impl From<&SearchResult> for Evidence {
    fn from(r: &SearchResult) -> Self {
        Evidence {
            id: r.hit.chunk_id.clone(),
            meeting_id: r.hit.meeting_id.clone(),
            meeting_title: r.hit.meeting_title.clone(),
            meeting_date: r.hit.meeting_date.clone(),
            kind: r.hit.kind.clone(),
            start_time: r.hit.start_time,
            end_time: r.hit.end_time,
            text: r.hit.text.clone(),
        }
    }
}

impl From<&FactRow> for Evidence {
    fn from(f: &FactRow) -> Self {
        let subject = f.ticket.as_deref().map(|t| format!(" {t}")).unwrap_or_default();
        let owner = f.owner.as_deref().map(|o| format!(" (owner: {o})")).unwrap_or_default();
        Evidence {
            id: f.id.clone(),
            meeting_id: f.meeting_id.clone(),
            meeting_title: f.meeting_title.clone(),
            meeting_date: f.meeting_date.clone(),
            kind: "fact".into(),
            start_time: f.start_time,
            end_time: None,
            text: format!("{}{}: {}{}", f.fact_type.to_uppercase(), subject, f.content, owner),
        }
    }
}

const ANSWER_SYSTEM: &str = "You answer questions about a team's meetings using ONLY the numbered excerpts provided. Rules:\n\
- Cite every factual claim with the excerpt number in square brackets, e.g. [2] or [1][3].\n\
- When the question is about when or in which meeting something was said, give the meeting title and date.\n\
- For status questions (e.g. whether something is blocked and why), prefer the most recent meeting and say when it was discussed.\n\
- If the excerpts do not contain the answer, say clearly that you could not find it in the meetings. Never invent facts.\n\
- Answer in the same language as the question. Be concise; markdown is allowed.";

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "summary" => "Summary",
        "notes" => "Notes",
        "fact" => "Recorded fact",
        _ => "Transcript",
    }
}

fn format_time(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn answer_prompt(
    question: &str,
    today: NaiveDate,
    project_context: Option<&str>,
    evidence: &[Evidence],
    history: &[ConversationTurn],
) -> String {
    let excerpts = evidence
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let date = e.meeting_date.get(..10).unwrap_or(&e.meeting_date);
            let time = e.start_time.map(|t| format!(" · {}", format_time(t))).unwrap_or_default();
            format!(
                "[{}] Meeting \"{}\" · {}{} · {}\n{}",
                i + 1,
                e.meeting_title,
                date,
                time,
                kind_label(&e.kind),
                e.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let history = format_history(history);
    let mut prompt = format!("Today is {today}.\n\n");
    if let Some(context) = project_context {
        prompt.push_str(&format!("<project>\n{context}\n</project>\n\n"));
    }
    if !history.is_empty() {
        prompt.push_str(&format!("<conversation>\n{history}\n</conversation>\n\n"));
    }
    prompt.push_str(&format!("<excerpts>\n{excerpts}\n</excerpts>\n\nQuestion: {question}"));
    prompt
}

/// Returns the excerpt numbers cited as [n] (also [1, 2]), in first-seen order.
pub fn cited_indices(answer: &str, max: usize) -> Vec<usize> {
    let mut cited = Vec::new();
    let mut rest = answer;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { break };
        for part in after[..close].split(',') {
            if let Ok(n) = part.trim().parse::<usize>() {
                if (1..=max).contains(&n) && !cited.contains(&n) {
                    cited.push(n);
                }
            }
        }
        rest = &after[close + 1..];
    }
    cited
}

/// Removes citation markers like [2] or [1, 3], leaving other brackets intact.
fn strip_citations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find(']') {
            Some(close)
                if !after[..close].trim().is_empty()
                    && after[..close].split(',').all(|p| p.trim().parse::<usize>().is_ok()) =>
            {
                rest = &after[close + 1..];
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace(" .", ".").trim().to_string()
}

fn excerpt(text: &str) -> String {
    const MAX: usize = 400;
    if text.chars().count() <= MAX {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(MAX).collect::<String>())
    }
}

pub async fn ask_project(
    pool: &SqlitePool,
    embedder: &dyn EmbeddingProvider,
    llm: &dyn ChatModel,
    project_id: Option<&str>,
    question: &str,
    history: &[ConversationTurn],
    today: NaiveDate,
) -> Result<Answer> {
    ask_project_with(pool, embedder, None, llm, project_id, question, history, today).await
}

/// `ask_project` with semantic search in Qdrant when configured.
#[allow(clippy::too_many_arguments)]
pub async fn ask_project_with(
    pool: &SqlitePool,
    embedder: &dyn EmbeddingProvider,
    qdrant: Option<&QdrantStore>,
    llm: &dyn ChatModel,
    project_id: Option<&str>,
    question: &str,
    history: &[ConversationTurn],
    today: NaiveDate,
) -> Result<Answer> {
    let question = question.trim();

    // 1. Plan
    let meetings = recent_meetings(pool, project_id).await?;
    let meeting_ids: Vec<String> = meetings.iter().map(|m| m.id.clone()).collect();
    // Across all projects, any project's ticket pattern counts
    let patterns: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT group_concat(ticket_patterns, char(10)) FROM projects
         WHERE (? IS NULL OR id = ?) AND ticket_patterns IS NOT NULL AND ticket_patterns != ''",
    )
    .bind(project_id)
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    let matcher = TicketMatcher::new(patterns.and_then(|(p,)| p).as_deref());
    let mut plan = match llm
        .complete(PLANNER_SYSTEM, &planner_prompt(question, today, &meetings, history))
        .await
    {
        Ok(reply) => parse_plan(&reply, question, &meeting_ids, &matcher),
        Err(e) => {
            log::warn!("RAG: planner failed, searching with the raw question: {}", e);
            QueryPlan { search_query: question.to_string(), ..Default::default() }
        }
    };
    // Ticket IDs typed in the question count even if the planner missed them
    plan.tickets = dedup(plan.tickets.iter().cloned().chain(matcher.find_all(question)));
    log::info!("RAG: plan {:?}", plan);

    // 2. Retrieve (relaxing filters if they leave nothing)
    let filters = SearchFilters {
        meeting_id: plan.meeting_id.clone(),
        date_from: plan.date_from.clone(),
        date_to: plan.date_to.clone(),
    };
    let has_filters = filters.meeting_id.is_some() || filters.date_from.is_some() || filters.date_to.is_some();
    let mut response =
        hybrid_search_with(pool, embedder, qdrant, project_id, &plan.search_query, &filters, PASSAGES_FOR_ANSWER)
            .await?;
    let mut filters_relaxed = false;
    if response.results.is_empty() && has_filters {
        filters_relaxed = true;
        response = hybrid_search_with(
            pool,
            embedder,
            qdrant,
            project_id,
            &plan.search_query,
            &SearchFilters::default(),
            PASSAGES_FOR_ANSWER,
        )
        .await?;
    }
    let notice = response.vector_error.clone();

    // Recorded facts: newest statements about the tickets asked about, plus
    // decisions/actions/etc. when the question is about those
    let mut facts = facts_for_tickets(pool, project_id, &plan.tickets, FACTS_FOR_ANSWER).await?;
    if !plan.fact_types.is_empty() {
        let (meeting, from, to) = if filters_relaxed {
            (None, None, None)
        } else {
            (plan.meeting_id.as_deref(), plan.date_from.as_deref(), plan.date_to.as_deref())
        };
        for fact in facts_by_type(pool, project_id, &plan.fact_types, meeting, from, to, FACTS_FOR_ANSWER).await? {
            if !facts.iter().any(|f| f.id == fact.id) {
                facts.push(fact);
            }
        }
        facts.truncate(FACTS_FOR_ANSWER);
    }
    let evidence: Vec<Evidence> = facts
        .iter()
        .map(Evidence::from)
        .chain(response.results.iter().map(Evidence::from))
        .collect();

    // 3. Evidence gate
    if evidence.is_empty() {
        return Ok(Answer {
            answer: String::new(),
            found: false,
            citations: Vec::new(),
            plan,
            filters_relaxed,
            notice,
            history_id: None,
        });
    }

    // 4. Answer
    // Across projects, name each passage's project so the answer can tell them apart
    let project_names = match project_id {
        Some(_) => HashMap::new(),
        None => meeting_project_names(pool, &evidence).await?,
    };
    let evidence: Vec<Evidence> = evidence
        .into_iter()
        .map(|mut e| {
            if let Some(name) = project_names.get(&e.meeting_id) {
                e.meeting_title = format!("[{name}] {}", e.meeting_title);
            }
            e
        })
        .collect();
    let project_context = match project_id {
        Some(id) => ProjectsRepository::project_context(pool, id).await?,
        None => None,
    };
    let answer = llm
        .complete(
            ANSWER_SYSTEM,
            &answer_prompt(question, today, project_context.as_deref(), &evidence, history),
        )
        .await?
        .trim()
        .to_string();

    let cited = cited_indices(&answer, evidence.len());
    let citations = evidence
        .iter()
        .enumerate()
        .filter(|(i, _)| cited.is_empty() || cited.contains(&(i + 1)))
        .map(|(i, e)| Citation {
            index: i + 1,
            chunk_id: e.id.clone(),
            meeting_id: e.meeting_id.clone(),
            meeting_title: match project_names.get(&e.meeting_id) {
                Some(name) => e.meeting_title.trim_start_matches(&format!("[{name}] ")).to_string(),
                None => e.meeting_title.clone(),
            },
            meeting_date: e.meeting_date.clone(),
            kind: e.kind.clone(),
            start_time: e.start_time,
            end_time: e.end_time,
            excerpt: excerpt(&e.text),
            project_name: project_names.get(&e.meeting_id).cloned(),
        })
        .collect();

    Ok(Answer { answer, found: true, citations, plan, filters_relaxed, notice, history_id: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rag::indexer::index_meeting;
    use anyhow::anyhow;
    use async_trait::async_trait;
    use chrono::Utc;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Mutex;

    struct KeywordEmbedder;

    #[async_trait]
    impl EmbeddingProvider for KeywordEmbedder {
        fn model_id(&self) -> &str {
            "kw"
        }
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            // One dimension per keyword plus an "other" dimension for texts with none,
            // so unrelated questions are orthogonal to every passage.
            const VOCAB: &[&str] = &["bloqueado", "acesso", "deploy", "orçamento"];
            Ok(texts
                .iter()
                .map(|t| {
                    let t = t.to_lowercase();
                    let mut v: Vec<f32> = VOCAB.iter().map(|w| t.matches(w).count() as f32).collect();
                    v.push(if v.iter().all(|x| *x == 0.0) { 1.0 } else { 0.0 });
                    v
                })
                .collect())
        }
    }

    /// Replies with scripted outputs and records the prompts it received.
    struct ScriptedLlm {
        replies: Mutex<Vec<Result<String, String>>>,
        prompts: Mutex<Vec<String>>,
    }

    impl ScriptedLlm {
        fn new(replies: Vec<Result<&str, &str>>) -> Self {
            Self {
                replies: Mutex::new(
                    replies.into_iter().rev().map(|r| r.map(str::to_string).map_err(str::to_string)).collect(),
                ),
                prompts: Mutex::new(Vec::new()),
            }
        }
        fn calls(&self) -> usize {
            self.prompts.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl ChatModel for ScriptedLlm {
        async fn complete(&self, _system: &str, user: &str) -> Result<String> {
            self.prompts.lock().unwrap().push(user.to_string());
            self.replies
                .lock()
                .unwrap()
                .pop()
                .unwrap_or_else(|| Err("no scripted reply".into()))
                .map_err(|e| anyhow!(e))
        }
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 29).unwrap()
    }

    async fn setup() -> SqlitePool {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let now = Utc::now();
        sqlx::query("INSERT INTO projects (id, name, context_md, created_at, updated_at) VALUES ('p1', 'Pagamentos', 'Time de pagamentos', ?, ?)")
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        for (id, title, date, text) in [
            ("m1", "Daily", "2026-09-22 10:00:00+00:00", "O ticket ABC-123 está bloqueado por falta de acesso ao ambiente"),
            ("m2", "Retro", "2026-09-28 10:00:00+00:00", "O deploy de sexta correu bem"),
        ] {
            sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, project_id) VALUES (?, ?, ?, ?, 'p1')")
                .bind(id)
                .bind(title)
                .bind(date)
                .bind(date)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time) VALUES (?, ?, ?, '00:00', 125.0, 130.0)")
                .bind(format!("{id}-t"))
                .bind(id)
                .bind(text)
                .execute(&pool)
                .await
                .unwrap();
            index_meeting(&pool, &KeywordEmbedder, id).await.unwrap();
        }
        pool
    }

    #[test]
    fn parse_plan_validates_fields() {
        let ids = vec!["m1".to_string()];
        let plan = parse_plan(
            "Sure:\n```json\n{\"search_query\": \"ABC-123 bloqueio\", \"date_from\": \"2026-09-21\", \"date_to\": \"2026-09-28\", \"meeting_id\": \"m1\"}\n```",
            "q",
            &ids,
            &TicketMatcher::new(None),
        );
        assert_eq!(plan.search_query, "ABC-123 bloqueio");
        assert_eq!(plan.date_from.as_deref(), Some("2026-09-21"));
        assert_eq!(plan.date_to.as_deref(), Some("2026-09-28"));
        assert_eq!(plan.meeting_id.as_deref(), Some("m1"));

        // Unknown meeting, bad date, inverted range and empty query are dropped
        let plan = parse_plan(
            r#"{"search_query": " ", "date_from": "22/09", "date_to": "2026-09-01", "meeting_id": "invented"}"#,
            "original question",
            &ids,
            &TicketMatcher::new(None),
        );
        assert_eq!(plan, QueryPlan { search_query: "original question".into(), date_to: Some("2026-09-01".into()), ..Default::default() });

        let plan = parse_plan(r#"{"date_from": "2026-09-10", "date_to": "2026-09-01"}"#, "q", &ids, &TicketMatcher::new(None));
        assert_eq!((plan.date_from, plan.date_to), (None, None));

        assert_eq!(parse_plan("no json here", "q", &ids, &TicketMatcher::new(None)).search_query, "q");

        let plan = parse_plan(
            r#"{"search_query": "x", "tickets": ["abc-123", "nope", "ABC-123"], "fact_types": ["Decision", "gossip"]}"#,
            "q",
            &ids,
            &TicketMatcher::new(Some("ABC-\\d+")),
        );
        assert_eq!(plan.tickets, vec!["ABC-123".to_string()]);
        assert_eq!(plan.fact_types, vec!["decision".to_string()]);
    }

    #[test]
    fn cited_indices_parses_brackets() {
        assert_eq!(cited_indices("Sim [2]. Também [1, 3][2] e [9] [x]", 3), vec![2, 1, 3]);
        assert!(cited_indices("sem citações", 3).is_empty());
    }

    #[test]
    fn strip_citations_keeps_other_brackets() {
        assert_eq!(strip_citations("Bloqueado [1][2, 3]. Ver [link] e [ ]"), "Bloqueado. Ver [link] e [ ]");
    }

    #[tokio::test]
    async fn answers_with_citations_from_planned_search() {
        let pool = setup().await;
        let llm = ScriptedLlm::new(vec![
            Ok(r#"{"search_query": "ABC-123 bloqueado", "date_from": null, "date_to": null, "meeting_id": null}"#),
            Ok("O ABC-123 está bloqueado por falta de acesso ao ambiente, discutido na Daily de 22/09 [1]."),
        ]);
        let answer = ask_project(&pool, &KeywordEmbedder, &llm, Some("p1"), "Por que o ABC-123 está bloqueado?", &[], today())
            .await
            .unwrap();

        assert!(answer.found);
        assert!(!answer.filters_relaxed);
        assert_eq!(answer.citations.len(), 1);
        let citation = &answer.citations[0];
        assert_eq!(citation.index, 1);
        assert_eq!(citation.meeting_id, "m1");
        assert_eq!(citation.start_time, Some(125.0));

        // The answer prompt carries project context, dated excerpts and timestamps
        let prompts = llm.prompts.lock().unwrap();
        assert!(prompts[0].contains("m1 | 2026-09-22 | Daily"));
        assert!(prompts[1].contains("Time de pagamentos"));
        assert!(prompts[1].contains("[1] Meeting \"Daily\" · 2026-09-22 · 2:05 · Transcript"));
    }

    #[tokio::test]
    async fn all_projects_search_names_each_passage_project() {
        let pool = setup().await;
        let now = Utc::now();
        sqlx::query("INSERT INTO projects (id, name, created_at, updated_at) VALUES ('p2', 'Logística', ?, ?)")
            .bind(now).bind(now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, project_id) VALUES ('m3', 'Sync', '2026-09-27 10:00:00+00:00', '2026-09-27 10:00:00+00:00', 'p2')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time) VALUES ('m3-t', 'm3', 'O deploy da logística atrasou', '00:00', 10.0, 12.0)")
            .execute(&pool).await.unwrap();
        index_meeting(&pool, &KeywordEmbedder, "m3").await.unwrap();

        let llm = ScriptedLlm::new(vec![
            Ok(r#"{"search_query": "deploy"}"#),
            Ok("Pagamentos fez o deploy [1]; na Logística atrasou [2]."),
        ]);
        let answer = ask_project(&pool, &KeywordEmbedder, &llm, None, "Como foram os deploys?", &[], today())
            .await
            .unwrap();
        assert!(answer.found);
        let mut projects: Vec<(String, Option<String>)> =
            answer.citations.iter().map(|c| (c.meeting_id.clone(), c.project_name.clone())).collect();
        projects.sort();
        assert_eq!(
            projects,
            vec![("m2".into(), Some("Pagamentos".into())), ("m3".into(), Some("Logística".into()))]
        );
        assert!(answer.citations.iter().all(|c| !c.meeting_title.starts_with('[')));
        let prompts = llm.prompts.lock().unwrap();
        assert!(prompts[1].contains("[Logística] Sync"));
        assert!(!prompts[1].contains("Time de pagamentos")); // no single-project context

        // Scoped to one project, the other one's meeting is not found
        let llm = ScriptedLlm::new(vec![Ok(r#"{"search_query": "logística"}"#), Ok("nada")]);
        let scoped = ask_project(&pool, &KeywordEmbedder, &llm, Some("p1"), "E a logística?", &[], today()).await.unwrap();
        assert!(scoped.citations.iter().all(|c| c.meeting_id != "m3" && c.project_name.is_none()));
    }

    #[tokio::test]
    async fn date_filter_is_applied_and_relaxed_when_empty() {
        let pool = setup().await;
        // Filter to the Retro's date: only m2 is eligible
        let llm = ScriptedLlm::new(vec![
            Ok(r#"{"search_query": "deploy", "date_from": "2026-09-28", "date_to": "2026-09-29"}"#),
            Ok("Correu bem [1]."),
        ]);
        let answer = ask_project(&pool, &KeywordEmbedder, &llm, Some("p1"), "Como foi o deploy ontem?", &[], today())
            .await
            .unwrap();
        assert!(!answer.filters_relaxed);
        assert!(answer.citations.iter().all(|c| c.meeting_id == "m2"));

        // A date range with no meetings falls back to an unfiltered search
        let llm = ScriptedLlm::new(vec![
            Ok(r#"{"search_query": "ABC-123", "date_from": "2025-01-01", "date_to": "2025-01-02"}"#),
            Ok("Bloqueado [1]."),
        ]);
        let answer = ask_project(&pool, &KeywordEmbedder, &llm, Some("p1"), "ABC-123?", &[], today()).await.unwrap();
        assert!(answer.filters_relaxed);
        assert!(answer.found);
    }

    #[tokio::test]
    async fn no_evidence_skips_the_answer_call() {
        let pool = setup().await;
        let llm = ScriptedLlm::new(vec![Ok(r#"{"search_query": "kubernetes"}"#)]);
        let answer = ask_project(&pool, &KeywordEmbedder, &llm, Some("p1"), "E o kubernetes?", &[], today()).await.unwrap();
        assert!(!answer.found);
        assert!(answer.citations.is_empty());
        assert_eq!(llm.calls(), 1); // planner only
    }

    #[tokio::test]
    async fn planner_failure_falls_back_to_raw_question() {
        let pool = setup().await;
        let llm = ScriptedLlm::new(vec![Err("timeout"), Ok("Foi bem [1].")]);
        let history = vec![ConversationTurn { question: "Teve deploy?".into(), answer: "Sim, sexta [2].".into() }];
        let answer = ask_project(&pool, &KeywordEmbedder, &llm, Some("p1"), "Como foi o deploy?", &history, today())
            .await
            .unwrap();
        assert_eq!(answer.plan.search_query, "Como foi o deploy?");
        assert!(answer.found);
        let prompts = llm.prompts.lock().unwrap();
        assert!(prompts[1].contains("Q: Teve deploy?\nA: Sim, sexta."));
        assert!(!prompts[1].contains("[2]"));
    }

    #[tokio::test]
    async fn recorded_facts_come_first_and_are_cited() {
        let pool = setup().await;
        let now = Utc::now();
        sqlx::query("INSERT INTO entities (id, project_id, entity_type, key, display_name, created_at, updated_at) VALUES ('e1', 'p1', 'ticket', 'ABC-123', 'ABC-123', ?, ?)")
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO entity_facts (id, entity_id, project_id, meeting_id, fact_type, content, start_time, meeting_date, created_at) VALUES ('f1', 'e1', 'p1', 'm1', 'blocker', 'Falta acesso ao ambiente de homologação', 125.0, '2026-09-22 10:00:00+00:00', ?)")
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO entity_facts (id, entity_id, project_id, meeting_id, fact_type, content, meeting_date, created_at) VALUES ('f2', NULL, 'p1', 'm2', 'decision', 'Release na sexta', '2026-09-28 10:00:00+00:00', ?)")
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();

        // The planner misses the ticket; it is still picked up from the question text
        let llm = ScriptedLlm::new(vec![
            Ok(r#"{"search_query": "bloqueio", "tickets": [], "fact_types": ["decision"]}"#),
            Ok("Falta acesso ao ambiente [1]. Também decidiram a release [2]."),
        ]);
        let answer = ask_project(&pool, &KeywordEmbedder, &llm, Some("p1"), "Por que o ABC-123 está bloqueado?", &[], today())
            .await
            .unwrap();
        assert_eq!(answer.plan.tickets, vec!["ABC-123".to_string()]);
        assert_eq!(answer.citations.len(), 2);
        assert_eq!(answer.citations[0].kind, "fact");
        assert_eq!(answer.citations[0].chunk_id, "f1");
        assert_eq!(answer.citations[1].chunk_id, "f2");
        let prompts = llm.prompts.lock().unwrap();
        assert!(prompts[1].contains("[1] Meeting \"Daily\" · 2026-09-22 · 2:05 · Recorded fact\nBLOCKER ABC-123: Falta acesso"));
    }
}

//! "My time": the tickets the user worked on in a period, from what they said
//! in meetings (their microphone lines, or the speaker recognized as them).
//!
//! 1. Meetings of the period (one project or all) → the user's lines.
//! 2. Tickets per line: mentioned by the user, or asked about right before the
//!    user answers; follow-up lines continue the same ticket for a while. This
//!    gives the talk time and the quotes per ticket.
//! 3. The configured LLM reads the quotes and writes a short description and an
//!    estimate of the hours of work ("ontem fiquei o dia todo no ABC-123").

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, HashMap};

use super::identity::{get_profile, label_voice_sources};
use crate::rag::entities::TicketMatcher;
use crate::rag::llm::ChatModel;

/// A question from someone else counts for the user's reply within this gap.
const QUESTION_GAP_SECONDS: f64 = 15.0;
/// The user's follow-up lines keep the ticket while gaps stay below this…
const FOLLOW_UP_GAP_SECONDS: f64 = 20.0;
/// …and for at most this long after the last mention.
const FOLLOW_UP_SPAN_SECONDS: f64 = 180.0;
const QUOTES_PER_TICKET: usize = 6;
const QUOTE_CHARS: usize = 220;
const MAX_TICKETS_FOR_LLM: usize = 30;
/// Hours of a working day, for the model's estimates.
const WORKDAY_HOURS: f64 = 8.0;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MyTimeRequest {
    /// Active project; ignored when `all_projects`.
    pub project_id: Option<String>,
    #[serde(default)]
    pub all_projects: bool,
    /// Inclusive dates, YYYY-MM-DD.
    pub date_from: String,
    pub date_to: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Quote {
    pub meeting_id: String,
    pub meeting_title: String,
    pub date: String,
    pub start_time: Option<f64>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyTicket {
    pub key: String,
    pub project_name: Option<String>,
    pub description: Option<String>,
    /// Estimated hours of work in the period (LLM), if any.
    pub estimated_hours: Option<f64>,
    pub estimate_basis: Option<String>,
    /// Seconds the user spoke about the ticket in meetings.
    pub talk_seconds: f64,
    /// Lines of the user naming the ticket.
    pub mentions: usize,
    pub meeting_count: usize,
    pub first_date: String,
    pub last_date: String,
    pub quotes: Vec<Quote>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyTimeReport {
    pub date_from: String,
    pub date_to: String,
    pub meetings: usize,
    /// Meetings where the user was recognized.
    pub meetings_with_me: usize,
    /// The user's lines, by how they were recognized.
    pub mic_lines: usize,
    pub voice_lines: usize,
    pub tickets: Vec<MyTicket>,
    pub total_estimated_hours: Option<f64>,
    pub total_talk_seconds: f64,
    /// Why the descriptions/estimates are missing (e.g. no language model).
    pub llm_error: Option<String>,
}

/// One transcript line, as far as the report is concerned.
#[derive(Debug, Clone)]
pub struct Line {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub mine: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TicketStats {
    pub talk_seconds: f64,
    pub mentions: usize,
    /// (start time, text) of the user's lines about the ticket.
    pub quotes: Vec<(f64, String)>,
}

/// Tickets the user talked about in one meeting's lines (in time order).
pub fn attribute(lines: &[Line], matcher: &TicketMatcher) -> BTreeMap<String, TicketStats> {
    let mut stats: BTreeMap<String, TicketStats> = BTreeMap::new();
    // Ticket(s) the user is on, since when they were last named, and the end of the user's last line
    let mut current: Vec<String> = Vec::new();
    let mut named_at = f64::NEG_INFINITY;
    let mut last_mine_end = f64::NEG_INFINITY;
    let mut previous_other: Option<(f64, Vec<String>)> = None;

    for line in lines {
        let named = matcher.find_all(&line.text);
        if !line.mine {
            if !named.is_empty() {
                previous_other = Some((line.end, named));
            }
            continue;
        }

        let (keys, is_mention) = if !named.is_empty() {
            (named, true)
        } else if let Some((end, asked)) = previous_other.take().filter(|(end, _)| line.start - end <= QUESTION_GAP_SECONDS) {
            let _ = end;
            (asked, true)
        } else if !current.is_empty()
            && line.start - last_mine_end <= FOLLOW_UP_GAP_SECONDS
            && line.start - named_at <= FOLLOW_UP_SPAN_SECONDS
        {
            (current.clone(), false)
        } else {
            (Vec::new(), false)
        };
        previous_other = None;
        last_mine_end = line.end;
        if keys.is_empty() {
            current.clear();
            continue;
        }
        if is_mention {
            named_at = line.start;
        }
        let share = (line.end - line.start).max(0.0) / keys.len() as f64;
        for key in &keys {
            let entry = stats.entry(key.clone()).or_default();
            entry.talk_seconds += share;
            if is_mention {
                entry.mentions += 1;
            }
            entry.quotes.push((line.start, line.text.trim().to_string()));
        }
        current = keys;
    }
    stats
}

#[derive(Debug, Deserialize)]
struct Estimate {
    key: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    hours: Option<f64>,
    #[serde(default)]
    basis: Option<String>,
}

pub fn parse_estimates(reply: &str) -> HashMap<String, Estimate> {
    #[derive(Deserialize)]
    struct Reply {
        #[serde(default)]
        tickets: Vec<Estimate>,
    }
    let Some((start, end)) = reply.find('{').zip(reply.rfind('}')).filter(|(s, e)| s < e) else {
        return HashMap::new();
    };
    serde_json::from_str::<Reply>(&reply[start..=end])
        .map(|r| r.tickets)
        .unwrap_or_default()
        .into_iter()
        .map(|mut e| {
            e.key = e.key.trim().to_uppercase();
            e.hours = e.hours.filter(|h| h.is_finite() && *h >= 0.0).map(|h| (h * 4.0).round() / 4.0);
            (e.key.clone(), e)
        })
        .collect()
}

fn system_prompt() -> String {
    format!(
        r#"You estimate how much work a person did on each ticket during a period, from what THEY said in meetings (their own lines, with dates). Typical evidence: stand-up updates ("yesterday I worked on ABC-123", "spent the morning on it", "finished ABC-7", "still blocked on…").

For each ticket:
- description: what the person did or is doing on it, at most 20 words, in the language of the quotes;
- hours: estimated hours of WORK in the period (not meeting time). A working day is {WORKDAY_HOURS} hours. Use explicit durations when given ("the morning" = 4, "all day" = 8). When several tickets were worked the same day without a split, divide the day between them. A ticket only mentioned or discussed, with no sign of work, gets 0;
- basis: at most 15 words on what the estimate rests on.

Reply with JSON only: {{"tickets":[{{"key":"ABC-123","description":"...","hours":6,"basis":"..."}}]}}"#
    )
}

fn user_prompt(date_from: &str, date_to: &str, name: Option<&str>, tickets: &[MyTicket]) -> String {
    let mut out = format!("Period: {date_from} to {date_to}\n");
    if let Some(name) = name {
        out.push_str(&format!("Person: {name}\n"));
    }
    for ticket in tickets.iter().take(MAX_TICKETS_FOR_LLM) {
        out.push_str(&format!("\nTicket {}", ticket.key));
        if let Some(project) = &ticket.project_name {
            out.push_str(&format!(" (project {project})"));
        }
        out.push_str(&format!(" — talked about it for {:.0} s:\n", ticket.talk_seconds));
        for quote in &ticket.quotes {
            out.push_str(&format!("- {} \"{}\": \"{}\"\n", quote.date, quote.meeting_title, quote.text));
        }
    }
    out
}

fn clip(text: &str) -> String {
    if text.chars().count() <= QUOTE_CHARS {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(QUOTE_CHARS).collect::<String>())
    }
}

pub async fn build_report(pool: &SqlitePool, llm: Option<&dyn ChatModel>, request: &MyTimeRequest) -> Result<MyTimeReport> {
    let project = if request.all_projects { None } else { request.project_id.as_deref() };
    let meetings: Vec<(String, String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT m.id, m.title, CAST(m.created_at AS TEXT), p.name, p.ticket_patterns
         FROM meetings m LEFT JOIN projects p ON p.id = m.project_id
         WHERE (? IS NULL OR m.project_id = ?)
           AND substr(CAST(m.created_at AS TEXT), 1, 10) >= ? AND substr(CAST(m.created_at AS TEXT), 1, 10) <= ?
         ORDER BY m.created_at",
    )
    .bind(project)
    .bind(project)
    .bind(&request.date_from)
    .bind(&request.date_to)
    .fetch_all(pool)
    .await?;

    let mut tickets: BTreeMap<String, MyTicket> = BTreeMap::new();
    let mut meetings_per_ticket: HashMap<String, Vec<String>> = HashMap::new();
    let (mut mic_lines, mut voice_lines, mut meetings_with_me) = (0, 0, 0);

    for (meeting_id, title, created_at, project_name, patterns) in &meetings {
        label_voice_sources(pool, meeting_id).await?;
        let rows: Vec<(String, Option<f64>, Option<f64>, Option<String>, Option<bool>)> = sqlx::query_as(
            "SELECT t.transcript, t.audio_start_time, t.audio_end_time, t.voice_source, s.is_me
             FROM transcripts t LEFT JOIN meeting_speakers s ON s.id = t.speaker_id
             WHERE t.meeting_id = ? ORDER BY COALESCE(t.audio_start_time, 0), t.timestamp",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        let mut any_mine = false;
        let lines: Vec<Line> = rows
            .into_iter()
            .map(|(text, start, end, source, is_me)| {
                let start = start.unwrap_or(0.0);
                let end = end.unwrap_or(start);
                // The mic track decides when it can; otherwise the recognized speaker
                let mine = match source.as_deref() {
                    Some("mic") => {
                        mic_lines += 1;
                        true
                    }
                    Some("system") => false,
                    _ if is_me == Some(true) => {
                        voice_lines += 1;
                        true
                    }
                    _ => false,
                };
                any_mine |= mine;
                Line { start, end, text, mine }
            })
            .collect();
        if !any_mine {
            continue;
        }
        meetings_with_me += 1;

        let date = created_at.get(..10).unwrap_or(created_at).to_string();
        let matcher = TicketMatcher::new(patterns.as_deref());
        for (key, stats) in attribute(&lines, &matcher) {
            let ticket = tickets.entry(key.clone()).or_insert_with(|| MyTicket {
                key: key.clone(),
                project_name: project_name.clone(),
                description: None,
                estimated_hours: None,
                estimate_basis: None,
                talk_seconds: 0.0,
                mentions: 0,
                meeting_count: 0,
                first_date: date.clone(),
                last_date: date.clone(),
                quotes: Vec::new(),
            });
            ticket.talk_seconds += stats.talk_seconds;
            ticket.mentions += stats.mentions;
            ticket.last_date = date.clone();
            let seen = meetings_per_ticket.entry(key.clone()).or_default();
            if !seen.contains(meeting_id) {
                seen.push(meeting_id.clone());
                ticket.meeting_count += 1;
            }
            for (start, text) in stats.quotes {
                ticket.quotes.push(Quote {
                    meeting_id: meeting_id.clone(),
                    meeting_title: title.clone(),
                    date: date.clone(),
                    start_time: Some(start),
                    text: clip(&text),
                });
            }
        }
    }

    // Keep the most recent quotes of each ticket
    let mut tickets: Vec<MyTicket> = tickets
        .into_values()
        .map(|mut t| {
            let skip = t.quotes.len().saturating_sub(QUOTES_PER_TICKET);
            t.quotes.drain(..skip);
            t
        })
        .collect();
    tickets.sort_by(|a, b| b.talk_seconds.partial_cmp(&a.talk_seconds).unwrap_or(std::cmp::Ordering::Equal));

    let mut llm_error = None;
    if let (Some(llm), false) = (llm, tickets.is_empty()) {
        let name = get_profile(pool).await?.display_name;
        match llm
            .complete(&system_prompt(), &user_prompt(&request.date_from, &request.date_to, name.as_deref(), &tickets))
            .await
        {
            Ok(reply) => {
                let estimates = parse_estimates(&reply);
                if estimates.is_empty() {
                    llm_error = Some("The language model did not return estimates".to_string());
                }
                for ticket in &mut tickets {
                    if let Some(estimate) = estimates.get(&ticket.key.to_uppercase()) {
                        ticket.description = estimate.description.clone().filter(|d| !d.trim().is_empty());
                        ticket.estimated_hours = estimate.hours;
                        ticket.estimate_basis = estimate.basis.clone().filter(|b| !b.trim().is_empty());
                    }
                }
            }
            Err(e) => llm_error = Some(e.to_string()),
        }
    } else if llm.is_none() && !tickets.is_empty() {
        llm_error = Some("No summary model configured".to_string());
    }

    // Most worked first when there are estimates
    tickets.sort_by(|a, b| {
        b.estimated_hours
            .unwrap_or(-1.0)
            .partial_cmp(&a.estimated_hours.unwrap_or(-1.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.talk_seconds.partial_cmp(&a.talk_seconds).unwrap_or(std::cmp::Ordering::Equal))
    });
    let hours: Vec<f64> = tickets.iter().filter_map(|t| t.estimated_hours).collect();
    Ok(MyTimeReport {
        date_from: request.date_from.clone(),
        date_to: request.date_to.clone(),
        meetings: meetings.len(),
        meetings_with_me,
        mic_lines,
        voice_lines,
        total_estimated_hours: if hours.is_empty() { None } else { Some(hours.iter().sum()) },
        total_talk_seconds: tickets.iter().map(|t| t.talk_seconds).sum(),
        tickets,
        llm_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::me::identity::tests::{line, meeting_with_track, pool};
    use async_trait::async_trait;

    fn l(start: f64, end: f64, mine: bool, text: &str) -> Line {
        Line { start, end, text: text.into(), mine }
    }

    #[test]
    fn tickets_follow_mentions_questions_and_follow_ups() {
        let matcher = TicketMatcher::new(None);
        let lines = vec![
            l(0.0, 5.0, true, "Ontem trabalhei no ABC-123, corrigindo o login."),
            l(6.0, 10.0, true, "Falta só o teste de integração."), // follow-up
            l(11.0, 14.0, false, "E o OPS-7?"),
            l(15.0, 18.0, true, "Ainda bloqueado, sem acesso."), // answer to the question
            l(19.0, 22.0, false, "Ok. Próximo."),
            l(60.0, 62.0, true, "Nada mais."), // too far: no ticket
            l(63.0, 70.0, true, "Hoje pego ABC-123 e XYZ-9."), // two tickets share the time
        ];
        let stats = attribute(&lines, &matcher);
        let abc = &stats["ABC-123"];
        assert_eq!(abc.mentions, 2);
        assert!((abc.talk_seconds - (5.0 + 4.0 + 3.5)).abs() < 1e-9);
        assert_eq!(abc.quotes.len(), 3);
        let ops = &stats["OPS-7"];
        assert_eq!((ops.mentions, ops.talk_seconds), (1, 3.0));
        assert!((stats["XYZ-9"].talk_seconds - 3.5).abs() < 1e-9);
        assert_eq!(stats.len(), 3);

        // Others talking about a ticket does not make it the user's
        let theirs = vec![l(0.0, 5.0, false, "Eu fiz o ABC-1."), l(40.0, 42.0, true, "Legal.")];
        assert!(attribute(&theirs, &matcher).is_empty());
    }

    #[test]
    fn estimates_are_parsed_and_rounded() {
        let reply = r#"Here: {"tickets":[{"key":"abc-123","description":"Corrigiu o login","hours":6.1,"basis":"disse que passou o dia"},{"key":"OPS-7","hours":-2}]}"#;
        let estimates = parse_estimates(reply);
        assert_eq!(estimates["ABC-123"].hours, Some(6.0));
        assert_eq!(estimates["OPS-7"].hours, None);
        assert!(parse_estimates("nope").is_empty());
    }

    struct Canned;
    #[async_trait]
    impl ChatModel for Canned {
        async fn complete(&self, system: &str, user: &str) -> anyhow::Result<String> {
            assert!(system.contains("working day is 8 hours"));
            assert!(user.contains("Person: Felipe"));
            assert!(user.contains("Ticket ABC-123"));
            assert!(!user.contains("Ticket OPS-9")); // only others said it
            Ok(r#"{"tickets":[{"key":"ABC-123","description":"Correção do login","hours":8,"basis":"disse que passou o dia"}]}"#.into())
        }
    }

    #[tokio::test]
    async fn report_uses_only_the_users_lines_in_period_and_scope() {
        let pool = pool().await;
        crate::me::identity::set_display_name(&pool, Some("Felipe")).await.unwrap();
        // In the period, project p1: mic (0–10 s) is the user, 10–20 s others
        meeting_with_track(&pool, "m1", "p1", "2026-09-22 10:00:00+00:00").await;
        line(&pool, "m1", 0, 1.0, "Ontem passei o dia no ABC-123.").await;
        line(&pool, "m1", 1, 4.0, "Deve fechar hoje.").await;
        line(&pool, "m1", 2, 12.0, "Eu cuidei do OPS-9.").await;
        // Other project, same period
        meeting_with_track(&pool, "m2", "p2", "2026-09-23 10:00:00+00:00").await;
        line(&pool, "m2", 0, 1.0, "Revisei o PAY-4.").await;
        // Outside the period
        meeting_with_track(&pool, "m3", "p1", "2026-10-05 10:00:00+00:00").await;
        line(&pool, "m3", 0, 1.0, "Comecei o ABC-200.").await;

        let request = MyTimeRequest {
            project_id: Some("p1".into()),
            all_projects: false,
            date_from: "2026-09-01".into(),
            date_to: "2026-09-30".into(),
        };
        let report = build_report(&pool, Some(&Canned), &request).await.unwrap();
        assert_eq!((report.meetings, report.meetings_with_me, report.mic_lines), (1, 1, 2));
        assert_eq!(report.tickets.len(), 1);
        let abc = &report.tickets[0];
        assert_eq!(abc.key, "ABC-123");
        assert_eq!(abc.estimated_hours, Some(8.0));
        assert_eq!(abc.description.as_deref(), Some("Correção do login"));
        assert!((abc.talk_seconds - 4.0).abs() < 1e-9); // mention + follow-up
        assert_eq!(abc.quotes.len(), 2);
        assert_eq!(report.total_estimated_hours, Some(8.0));

        let all = MyTimeRequest { all_projects: true, ..request };
        let report = build_report(&pool, None, &all).await.unwrap();
        let keys: Vec<&str> = report.tickets.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"ABC-123") && keys.contains(&"PAY-4"));
        assert_eq!(report.llm_error.as_deref(), Some("No summary model configured"));
    }
}

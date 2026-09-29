//! Speaker names from what was said: introductions ("aqui é o Bruno"), people
//! being addressed right before they answer ("Ana, what do you think?"),
//! thanks by name. One call to the configured summary LLM per meeting.
//!
//! Guesses only name speakers that are still unnamed (or were guessed before);
//! they never override the user or a voice recognition, and never teach
//! voiceprints. Two labels given the same name confidently are merged, which
//! fixes the most common diarization error: one person split in two.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;

use super::store::{MeetingSpeaker, SpeakerStore};
use crate::rag::llm::ChatModel;

/// Guesses below this are ignored.
const MIN_CONFIDENCE: f32 = 0.6;
/// Two speakers are merged only when both guesses are at least this sure.
const MERGE_CONFIDENCE: f32 = 0.75;
/// Transcript characters sent to the LLM.
const MAX_TRANSCRIPT_CHARS: usize = 24_000;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct NameGuess {
    pub label: String,
    pub name: String,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub evidence: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamingOutcome {
    pub named: usize,
    pub merged: usize,
}

const SYSTEM_PROMPT: &str = r#"You identify the speakers of a meeting transcript. Speakers were separated automatically by voice and labeled "Speaker 1", "Speaker 2", etc.; the labels can be wrong occasionally.

Find each speaker's real name using ONLY evidence in the text:
- self-introductions ("I'm Ana", "aqui é o Bruno falando");
- someone addressed by name right before that speaker answers ("Bruno, can you check?" followed by a reply);
- being thanked or answered by name ("thanks, Carla").
A name mentioned about someone else ("I talked to Pedro yesterday") is NOT evidence. Do not guess without evidence.
If two labels are clearly the same person, give both the same name.
When the evidence matches a known project member, use that member's name exactly.

Reply with JSON only:
{"speakers":[{"label":"Speaker 1","name":"Ana","confidence":0.9,"evidence":"short quote from the transcript"}]}
Omit speakers you cannot identify. confidence is between 0 and 1."#;

/// Transcript lines as "[label] text", keeping the start of the meeting (where
/// introductions happen) and sampling the rest evenly when it is too long.
pub fn transcript_excerpt(lines: &[(String, String)], max_chars: usize) -> String {
    let formatted: Vec<String> = lines
        .iter()
        .filter(|(_, text)| !text.trim().is_empty())
        .map(|(label, text)| format!("[{label}] {}", text.trim()))
        .collect();
    let total: usize = formatted.iter().map(|l| l.len() + 1).sum();
    if total <= max_chars {
        return formatted.join("\n");
    }

    let mut out = Vec::new();
    let mut used = 0;
    let mut i = 0;
    while i < formatted.len() && used + formatted[i].len() < max_chars / 2 {
        used += formatted[i].len() + 1;
        out.push(formatted[i].clone());
        i += 1;
    }
    out.push("[…]".to_string());
    let rest = &formatted[i..];
    let rest_chars: usize = rest.iter().map(|l| l.len() + 1).sum();
    let step = (rest_chars as f64 / (max_chars - used) as f64).ceil().max(1.0) as usize;
    for line in rest.iter().step_by(step) {
        if used + line.len() >= max_chars {
            break;
        }
        used += line.len() + 1;
        out.push(line.clone());
    }
    out.join("\n")
}

pub fn build_prompt(speakers: &[MeetingSpeaker], members: &[String], excerpt: &str) -> String {
    let labels: Vec<&str> = speakers.iter().map(|s| s.label.as_str()).collect();
    let members = if members.is_empty() { "(none registered)".to_string() } else { members.join(", ") };
    format!("Speaker labels: {}\nKnown project members: {}\n\nTranscript:\n{}", labels.join(", "), members, excerpt)
}

pub fn parse_guesses(reply: &str) -> Vec<NameGuess> {
    #[derive(Deserialize)]
    struct Reply {
        #[serde(default)]
        speakers: Vec<NameGuess>,
    }
    let Some((start, end)) = reply.find('{').zip(reply.rfind('}')).filter(|(s, e)| s < e) else {
        return Vec::new();
    };
    serde_json::from_str::<Reply>(&reply[start..=end])
        .map(|r| r.speakers)
        .unwrap_or_default()
        .into_iter()
        .map(|g| NameGuess { name: g.name.trim().to_string(), label: g.label.trim().to_string(), ..g })
        .filter(|g| !g.name.is_empty() && g.confidence >= MIN_CONFIDENCE && !g.name.to_lowercase().starts_with("speaker"))
        .collect()
}

/// The project member a guessed name refers to: exact name, else a unique
/// member with the same first name.
pub fn match_member<'a>(name: &str, members: &'a [(String, String)]) -> Option<&'a str> {
    let wanted = name.trim().to_lowercase();
    if let Some((id, _)) = members.iter().find(|(_, n)| n.trim().to_lowercase() == wanted) {
        return Some(id);
    }
    let first = wanted.split_whitespace().next()?;
    let mut same_first = members
        .iter()
        .filter(|(_, n)| n.split_whitespace().next().map(str::to_lowercase).as_deref() == Some(first));
    match (same_first.next(), same_first.next()) {
        (Some((id, _)), None) => Some(id),
        _ => None,
    }
}

/// What to do with the guesses: (speaker id, name, evidence) to apply, and
/// (into, from) speaker pairs to merge.
pub fn plan_naming(speakers: &[MeetingSpeaker], guesses: &[NameGuess]) -> (Vec<(String, String, String)>, Vec<(String, String)>) {
    let editable = |s: &MeetingSpeaker| matches!(s.name_source.as_deref(), None | Some("inferred"));
    let mut by_name: HashMap<String, Vec<(&MeetingSpeaker, &NameGuess)>> = HashMap::new();
    for guess in guesses {
        if let Some(speaker) = speakers.iter().find(|s| s.label.eq_ignore_ascii_case(&guess.label)) {
            by_name.entry(guess.name.to_lowercase()).or_default().push((speaker, guess));
        }
    }

    let mut names = Vec::new();
    let mut merges = Vec::new();
    let mut groups: Vec<_> = by_name.into_values().collect();
    groups.sort_by(|a, b| a[0].0.label.cmp(&b[0].0.label));
    for mut group in groups {
        // The speaker who talked most keeps the identity
        group.sort_by(|a, b| b.0.speaking_seconds.partial_cmp(&a.0.speaking_seconds).unwrap_or(std::cmp::Ordering::Equal));
        let (keeper, guess) = group[0];
        if editable(keeper) {
            names.push((keeper.id.clone(), guess.name.clone(), guess.evidence.clone()));
        }
        let all_sure = group.iter().all(|(_, g)| g.confidence >= MERGE_CONFIDENCE);
        for (other, _) in group.iter().skip(1) {
            if all_sure && editable(other) {
                merges.push((keeper.id.clone(), other.id.clone()));
            }
        }
    }
    (names, merges)
}

/// Names (and merges) a meeting's speakers from its transcript.
pub async fn infer_speaker_names(pool: &SqlitePool, llm: &dyn ChatModel, meeting_id: &str) -> Result<NamingOutcome> {
    let speakers = SpeakerStore::list(pool, meeting_id).await?;
    if !speakers.iter().any(|s| matches!(s.name_source.as_deref(), None | Some("inferred"))) {
        return Ok(NamingOutcome::default());
    }
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT transcript, speaker_id FROM transcripts WHERE meeting_id = ?
         ORDER BY COALESCE(audio_start_time, 0), timestamp",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await?;
    let label_of: HashMap<&str, &str> = speakers.iter().map(|s| (s.id.as_str(), s.label.as_str())).collect();
    let lines: Vec<(String, String)> = rows
        .into_iter()
        .map(|(text, speaker)| {
            let label = speaker.as_deref().and_then(|id| label_of.get(id).copied()).unwrap_or("?");
            (label.to_string(), text)
        })
        .collect();
    if lines.is_empty() {
        return Ok(NamingOutcome::default());
    }

    let members: Vec<(String, String)> = sqlx::query_as(
        "SELECT pm.id, pm.name FROM project_members pm JOIN meetings m ON m.project_id = pm.project_id WHERE m.id = ?",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await?;
    let member_names: Vec<String> = members.iter().map(|(_, n)| n.clone()).collect();

    let prompt = build_prompt(&speakers, &member_names, &transcript_excerpt(&lines, MAX_TRANSCRIPT_CHARS));
    let reply = llm.complete(SYSTEM_PROMPT, &prompt).await?;
    let guesses = parse_guesses(&reply);
    log::info!("Diarization: {} name guess(es) for meeting {}", guesses.len(), meeting_id);

    let (names, merges) = plan_naming(&speakers, &guesses);
    let mut outcome = NamingOutcome::default();
    for (into, from) in &merges {
        SpeakerStore::merge(pool, into, from).await?;
        outcome.merged += 1;
    }
    for (speaker_id, name, evidence) in &names {
        let member = match_member(name, &members);
        let display = member
            .and_then(|id| members.iter().find(|(m, _)| m == id).map(|(_, n)| n.as_str()))
            .unwrap_or(name);
        if SpeakerStore::apply_inferred_name(pool, speaker_id, display, member, evidence).await? {
            outcome.named += 1;
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diarization::store::NewSpeaker;
    use async_trait::async_trait;
    use sqlx::sqlite::SqlitePoolOptions;

    fn speaker(id: &str, label: &str, seconds: f64, source: Option<&str>) -> MeetingSpeaker {
        MeetingSpeaker {
            id: id.into(),
            label: label.into(),
            member_id: None,
            member_name: None,
            speaking_seconds: seconds,
            name_source: source.map(Into::into),
            name_evidence: None,
        }
    }

    fn guess(label: &str, name: &str, confidence: f32) -> NameGuess {
        NameGuess { label: label.into(), name: name.into(), confidence, evidence: "q".into() }
    }

    #[test]
    fn replies_are_parsed_and_weak_guesses_dropped() {
        let reply = r#"Sure: {"speakers":[{"label":"Speaker 1","name":" Ana ","confidence":0.9,"evidence":"sou a Ana"},
            {"label":"Speaker 2","name":"Bruno","confidence":0.3},{"label":"Speaker 3","name":"Speaker 3","confidence":1}]} done"#;
        assert_eq!(parse_guesses(reply), vec![NameGuess { label: "Speaker 1".into(), name: "Ana".into(), confidence: 0.9, evidence: "sou a Ana".into() }]);
        assert!(parse_guesses("no json").is_empty());
    }

    #[test]
    fn members_match_by_full_or_unique_first_name() {
        let members = vec![("a".to_string(), "Ana Souza".to_string()), ("b1".into(), "Bruno Lima".into()), ("b2".into(), "Bruno Costa".into())];
        assert_eq!(match_member("ana souza", &members), Some("a"));
        assert_eq!(match_member("Ana", &members), Some("a"));
        assert_eq!(match_member("Bruno", &members), None); // ambiguous
        assert_eq!(match_member("Carla", &members), None);
    }

    #[test]
    fn same_name_merges_into_the_main_speaker_and_manual_names_are_kept() {
        let speakers = vec![
            speaker("s1", "Speaker 1", 10.0, None),
            speaker("s2", "Speaker 2", 60.0, None),
            speaker("s3", "Speaker 3", 30.0, Some("manual")),
            speaker("s4", "Speaker 4", 5.0, None),
        ];
        let guesses = vec![guess("Speaker 1", "Ana", 0.9), guess("Speaker 2", "ana", 0.8), guess("Speaker 3", "Bruno", 0.9), guess("Speaker 4", "Carla", 0.65)];
        let (names, merges) = plan_naming(&speakers, &guesses);
        assert_eq!(merges, vec![("s2".to_string(), "s1".to_string())]);
        let named: Vec<&str> = names.iter().map(|(id, _, _)| id.as_str()).collect();
        assert_eq!(named, vec!["s2", "s4"]); // s3 was named by the user

        // Unsure about one of them → named, not merged
        let (_, merges) = plan_naming(&speakers, &[guess("Speaker 1", "Ana", 0.9), guess("Speaker 2", "Ana", 0.6)]);
        assert!(merges.is_empty());
    }

    #[test]
    fn long_transcripts_keep_the_start_and_sample_the_rest() {
        let lines: Vec<(String, String)> = (0..1000).map(|i| ("Speaker 1".to_string(), format!("line {i} {}", "x".repeat(40)))).collect();
        let excerpt = transcript_excerpt(&lines, 5_000);
        assert!(excerpt.len() <= 5_000);
        assert!(excerpt.starts_with("[Speaker 1] line 0 "));
        assert!(excerpt.contains("[…]"));
        assert!(excerpt.contains("line 9") && excerpt.lines().count() > 60);
    }

    struct Canned(String);
    #[async_trait]
    impl ChatModel for Canned {
        async fn complete(&self, _: &str, user: &str) -> Result<String> {
            assert!(user.contains("Known project members: Ana Souza"));
            assert!(user.contains("[Speaker 2] Oi, aqui é a Ana."));
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn names_are_applied_and_split_speakers_merged() {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for sql in [
            "INSERT INTO projects (id, name, created_at, updated_at) VALUES ('p1', 'P', 'x', 'x')",
            "INSERT INTO project_members (id, project_id, name, created_at, updated_at) VALUES ('ana', 'p1', 'Ana Souza', 'x', 'x')",
            "INSERT INTO meetings (id, title, created_at, updated_at, project_id) VALUES ('m1', 'Daily', 'x', 'x', 'p1')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        let texts = ["Bom dia, Ana, pode começar?", "Oi, aqui é a Ana.", "Obrigado, Bruno.", "Continuando…"];
        for (i, text) in texts.iter().enumerate() {
            sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time) VALUES (?, 'm1', ?, '0', ?)")
                .bind(format!("t{i}")).bind(*text).bind(i as f64).execute(&pool).await.unwrap();
        }
        let speakers = vec![
            NewSpeaker { centroid: vec![1.0, 0.0], speaking_seconds: 20.0 },
            NewSpeaker { centroid: vec![0.0, 1.0], speaking_seconds: 30.0 },
            NewSpeaker { centroid: vec![0.1, 0.9], speaking_seconds: 5.0 },
        ];
        let assignments = [("t0", 0), ("t1", 1), ("t2", 1), ("t3", 2)].map(|(t, s)| (t.to_string(), Some(s)));
        SpeakerStore::save(&pool, "m1", &speakers, &assignments).await.unwrap();

        let llm = Canned(r#"{"speakers":[{"label":"Speaker 1","name":"Bruno","confidence":0.8,"evidence":"Obrigado, Bruno"},
            {"label":"Speaker 2","name":"Ana","confidence":0.95,"evidence":"aqui é a Ana"},
            {"label":"Speaker 3","name":"Ana","confidence":0.8,"evidence":"continues her point"}]}"#.into());
        let outcome = infer_speaker_names(&pool, &llm, "m1").await.unwrap();
        assert_eq!((outcome.named, outcome.merged), (2, 1));

        let listed = SpeakerStore::list(&pool, "m1").await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].label, "Bruno");
        assert_eq!(listed[0].member_id, None);
        assert_eq!(listed[1].display_name(), "Ana Souza");
        assert_eq!(listed[1].member_id.as_deref(), Some("ana"));
        assert_eq!(listed[1].name_source.as_deref(), Some("inferred"));
        assert!((listed[1].speaking_seconds - 35.0).abs() < 1e-9);
        let (moved,): (Option<String>,) = sqlx::query_as("SELECT speaker_id FROM transcripts WHERE id = 't3'").fetch_one(&pool).await.unwrap();
        assert_eq!(moved, Some(listed[1].id.clone()));
        // A guess teaches no voice
        let (samples,): (i64,) = sqlx::query_as("SELECT voiceprint_samples FROM project_members WHERE id = 'ana'").fetch_one(&pool).await.unwrap();
        assert_eq!(samples, 0);

        // The user confirming the guessed member does teach it
        SpeakerStore::update(&pool, &listed[1].id, None, Some("ana")).await.unwrap();
        let (samples,): (i64,) = sqlx::query_as("SELECT voiceprint_samples FROM project_members WHERE id = 'ana'").fetch_one(&pool).await.unwrap();
        assert_eq!(samples, 1);
    }
}

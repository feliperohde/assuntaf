//! Optional Qdrant vector store (REST API). When configured, each meeting's
//! passage vectors are also written to Qdrant and semantic search queries it;
//! passage text, keyword search and a local copy of the vectors stay in SQLite,
//! so search falls back to the local vectors if Qdrant is unreachable.
//!
//! One collection per embedding model (vectors of different models have
//! different sizes and are never comparable): `<prefix>_<model slug>`.

use anyhow::{anyhow, Context, Result};
use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

use super::store::SearchFilters;

pub const DEFAULT_COLLECTION_PREFIX: &str = "assunta";
const UPSERT_BATCH: usize = 64;

/// A passage vector to store.
pub struct QdrantPoint<'a> {
    pub chunk_id: &'a str,
    pub vector: &'a [f32],
    pub project_id: &'a str,
    pub meeting_id: &'a str,
    pub meeting_date: &'a str,
}

/// Result of checking a Qdrant server.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QdrantProbe {
    pub url: String,
    pub reachable: bool,
    pub collections: Vec<String>,
    pub error: Option<String>,
}

pub struct QdrantStore {
    client: reqwest::Client,
    url: String,
    api_key: Option<String>,
    collection: String,
}

/// "bge-m3:latest" → "bge_m3_latest"
fn slug(model: &str) -> String {
    let mut out: String = model
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

pub fn normalize_url(url: &str) -> String {
    let url = url.trim();
    let with_scheme = if url.contains("://") { url.to_string() } else { format!("http://{url}") };
    match url::Url::parse(&with_scheme) {
        Ok(mut parsed) => {
            if parsed.port().is_none() && !url.contains("://") {
                let _ = parsed.set_port(Some(6333));
            }
            parsed.to_string().trim_end_matches('/').to_string()
        }
        Err(_) => with_scheme.trim_end_matches('/').to_string(),
    }
}

/// Qdrant point ids must be integers or UUIDs; chunk ids are "chunk-<uuid>".
fn point_id(chunk_id: &str) -> String {
    let raw = chunk_id.strip_prefix("chunk-").unwrap_or(chunk_id);
    match uuid::Uuid::parse_str(raw) {
        Ok(id) => id.to_string(),
        // Not a UUID (should not happen): derive a stable one from the text
        Err(_) => {
            let mut bytes = [0u8; 16];
            for (i, b) in chunk_id.bytes().enumerate() {
                bytes[i % 16] = bytes[i % 16].wrapping_mul(31).wrapping_add(b);
            }
            uuid::Builder::from_random_bytes(bytes).into_uuid().to_string()
        }
    }
}

/// Meeting date ("2026-09-22 10:00:00+00:00", RFC 3339 or "YYYY-MM-DD") as unix seconds.
fn timestamp(date: &str) -> Option<i64> {
    let date = date.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(date) {
        return Some(dt.timestamp());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_str(date, "%Y-%m-%d %H:%M:%S%.f%:z") {
        return Some(dt.timestamp());
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(date, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(Utc.from_utc_datetime(&dt).timestamp());
    }
    NaiveDate::parse_from_str(date.get(..10)?, "%Y-%m-%d")
        .ok()
        .map(|d| Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap()).timestamp())
}

/// Qdrant filter for a project (None = all) and the search filters.
fn search_filter(project_id: Option<&str>, filters: &SearchFilters) -> Value {
    let mut must = Vec::new();
    if let Some(project_id) = project_id {
        must.push(json!({ "key": "project_id", "match": { "value": project_id } }));
    }
    if let Some(meeting_id) = &filters.meeting_id {
        must.push(json!({ "key": "meeting_id", "match": { "value": meeting_id } }));
    }
    let mut range = serde_json::Map::new();
    if let Some(from) = filters.date_from.as_deref().and_then(timestamp) {
        range.insert("gte".into(), json!(from));
    }
    if let Some(to) = filters.date_to.as_deref().and_then(timestamp) {
        range.insert("lt".into(), json!(to));
    }
    if !range.is_empty() {
        must.push(json!({ "key": "meeting_ts", "range": range }));
    }
    json!({ "must": must })
}

impl QdrantStore {
    pub fn new(url: &str, api_key: Option<&str>, collection_prefix: Option<&str>, embedding_model: &str) -> Self {
        let url = normalize_url(url);
        let prefix = collection_prefix.map(str::trim).filter(|p| !p.is_empty()).unwrap_or(DEFAULT_COLLECTION_PREFIX);
        Self {
            client: crate::net::client_for(&url),
            api_key: api_key.map(str::trim).filter(|k| !k.is_empty()).map(String::from),
            collection: format!("{}_{}", slug(prefix), slug(embedding_model)),
            url,
        }
    }

    pub fn collection(&self) -> &str {
        &self.collection
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let request = self.client.request(method, format!("{}{}", self.url, path)).timeout(Duration::from_secs(30));
        match &self.api_key {
            Some(key) => request.header("api-key", key),
            None => request,
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder, action: &str) -> Result<Value> {
        let response = request
            .send()
            .await
            .map_err(|e| anyhow!("Cannot reach Qdrant at {}: {}", self.url, root_cause(&e)))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let detail = body
                .pointer("/status/error")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| body.to_string());
            return Err(anyhow!("Qdrant {} failed (HTTP {}): {}", action, status.as_u16(), detail));
        }
        Ok(body)
    }

    /// Creates the collection (cosine distance) and payload indexes if missing.
    async fn ensure_collection(&self, dims: usize) -> Result<()> {
        let path = format!("/collections/{}", self.collection);
        let existing = self.request(reqwest::Method::GET, &path).send().await;
        if let Ok(response) = existing {
            if response.status().is_success() {
                let body: Value = response.json().await.unwrap_or(Value::Null);
                let size = body.pointer("/result/config/params/vectors/size").and_then(Value::as_u64);
                if let Some(size) = size.filter(|&s| s as usize != dims) {
                    return Err(anyhow!(
                        "Qdrant collection {} holds {}-dimension vectors but the model produces {}; use another collection prefix",
                        self.collection,
                        size,
                        dims
                    ));
                }
                return Ok(());
            }
        }
        self.send(
            self.request(reqwest::Method::PUT, &path)
                .json(&json!({ "vectors": { "size": dims, "distance": "Cosine" } })),
            "create collection",
        )
        .await?;
        for (field, schema) in [("project_id", "keyword"), ("meeting_id", "keyword"), ("meeting_ts", "integer")] {
            self.send(
                self.request(reqwest::Method::PUT, &format!("{path}/index?wait=true"))
                    .json(&json!({ "field_name": field, "field_schema": schema })),
                "create payload index",
            )
            .await?;
        }
        Ok(())
    }

    /// Replaces a meeting's points with `points` (all of the same meeting).
    pub async fn replace_meeting(&self, meeting_id: &str, points: &[QdrantPoint<'_>]) -> Result<()> {
        if let Some(first) = points.first() {
            self.ensure_collection(first.vector.len()).await?;
        }
        self.delete_meeting(meeting_id).await?;
        for batch in points.chunks(UPSERT_BATCH) {
            let body: Vec<Value> = batch
                .iter()
                .map(|p| {
                    json!({
                        "id": point_id(p.chunk_id),
                        "vector": p.vector,
                        "payload": {
                            "chunk_id": p.chunk_id,
                            "project_id": p.project_id,
                            "meeting_id": p.meeting_id,
                            "meeting_ts": timestamp(p.meeting_date),
                        }
                    })
                })
                .collect();
            self.send(
                self.request(reqwest::Method::PUT, &format!("/collections/{}/points?wait=true", self.collection))
                    .json(&json!({ "points": body })),
                "upsert",
            )
            .await?;
        }
        Ok(())
    }

    /// Removes a meeting's points; a missing collection counts as done.
    pub async fn delete_meeting(&self, meeting_id: &str) -> Result<()> {
        let result = self
            .send(
                self.request(reqwest::Method::POST, &format!("/collections/{}/points/delete?wait=true", self.collection))
                    .json(&json!({ "filter": { "must": [{ "key": "meeting_id", "match": { "value": meeting_id } }] } })),
                "delete",
            )
            .await;
        match result {
            Err(e) if e.to_string().contains("HTTP 404") => Ok(()),
            other => other.map(|_| ()),
        }
    }

    /// Chunk ids and cosine scores of the `k` nearest passages above `min_similarity`.
    pub async fn search(
        &self,
        vector: &[f32],
        project_id: Option<&str>,
        filters: &SearchFilters,
        k: usize,
        min_similarity: f64,
    ) -> Result<Vec<(String, f64)>> {
        #[derive(Deserialize)]
        struct Hit {
            score: f64,
            payload: Option<Value>,
        }
        #[derive(Deserialize)]
        struct Response {
            result: Vec<Hit>,
        }
        let body = self
            .send(
                self.request(reqwest::Method::POST, &format!("/collections/{}/points/search", self.collection))
                    .json(&json!({
                        "vector": vector,
                        "limit": k,
                        "score_threshold": min_similarity,
                        "with_payload": ["chunk_id"],
                        "filter": search_filter(project_id, filters),
                    })),
                "search",
            )
            .await
            .or_else(|e| {
                // Nothing indexed into this collection yet
                if e.to_string().contains("HTTP 404") {
                    Ok(json!({ "result": [] }))
                } else {
                    Err(e)
                }
            })?;
        let response: Response = serde_json::from_value(body).context("Unexpected Qdrant search response")?;
        Ok(response
            .result
            .into_iter()
            .filter_map(|hit| {
                let chunk_id = hit.payload?.get("chunk_id")?.as_str()?.to_string();
                Some((chunk_id, hit.score))
            })
            .collect())
    }
}

fn root_cause(e: &reqwest::Error) -> String {
    let mut source: &dyn std::error::Error = e;
    while let Some(next) = source.source() {
        source = next;
    }
    source.to_string()
}

/// Lists the server's collections (GET /collections) to check URL and API key.
pub async fn probe(url: &str, api_key: Option<&str>) -> QdrantProbe {
    let store = QdrantStore::new(url, api_key, None, "probe");
    let result = store.send(store.request(reqwest::Method::GET, "/collections"), "list collections").await;
    match result {
        Ok(body) => QdrantProbe {
            url: store.url.clone(),
            reachable: true,
            collections: body
                .pointer("/result/collections")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|c| c.get("name").and_then(Value::as_str).map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            error: None,
        },
        Err(e) => QdrantProbe { url: store.url.clone(), reachable: false, collections: Vec::new(), error: Some(e.to_string()) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_names_and_urls() {
        let store = QdrantStore::new("192.168.3.16", Some(" "), None, "bge-m3:latest");
        assert_eq!(store.collection(), "assunta_bge_m3_latest");
        assert_eq!(store.url, "http://192.168.3.16:6333");
        assert!(store.api_key.is_none());
        assert_eq!(normalize_url("https://q.example.com/"), "https://q.example.com");
        assert_eq!(QdrantStore::new("x", None, Some("Team KB"), "m").collection(), "team_kb_m");
    }

    #[test]
    fn point_ids_are_uuids() {
        let id = uuid::Uuid::new_v4();
        assert_eq!(point_id(&format!("chunk-{id}")), id.to_string());
        let derived = point_id("weird-id");
        assert!(uuid::Uuid::parse_str(&derived).is_ok());
        assert_eq!(derived, point_id("weird-id"));
    }

    #[test]
    fn dates_and_filters() {
        assert_eq!(timestamp("2026-09-22 00:00:00+00:00"), timestamp("2026-09-22"));
        assert!(timestamp("2026-09-22T10:00:00Z").unwrap() > timestamp("2026-09-22").unwrap());
        assert!(timestamp("garbage").is_none());

        let filters = SearchFilters { meeting_id: None, date_from: Some("2026-09-01".into()), date_to: None };
        let filter = search_filter(Some("p1"), &filters);
        let must = filter["must"].as_array().unwrap();
        assert_eq!(must.len(), 2);
        assert_eq!(must[0]["match"]["value"], "p1");
        assert_eq!(must[1]["range"]["gte"], timestamp("2026-09-01").unwrap());
        assert!(search_filter(None, &SearchFilters::default())["must"].as_array().unwrap().is_empty());
    }
}

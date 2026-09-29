//! Embedding providers. Only Ollama for now (local, private); the trait keeps
//! room for other providers without touching the indexer or retriever.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";
/// Texts sent per request; keeps request bodies and latency bounded.
const BATCH_SIZE: usize = 16;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Identifier stored with each vector; vectors from different models are never compared.
    fn model_id(&self) -> &str;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

pub struct OllamaEmbedder {
    client: reqwest::Client,
    endpoint: String,
    model: String,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl OllamaEmbedder {
    pub fn new(endpoint: Option<&str>, model: &str) -> Self {
        let endpoint = endpoint
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .unwrap_or(DEFAULT_OLLAMA_ENDPOINT)
            .trim_end_matches('/')
            .to_string();
        Self {
            client: reqwest::Client::new(),
            endpoint,
            model: model.to_string(),
        }
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/api/embed", self.endpoint);
        let response = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(120))
            .json(&EmbedRequest { model: &self.model, input: texts })
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    anyhow!("Cannot connect to Ollama at {}. Is it running?", self.endpoint)
                } else {
                    anyhow!("Ollama embedding request failed: {}", e)
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            if status.as_u16() == 404 || body.contains("not found") {
                return Err(anyhow!(
                    "Embedding model '{}' is not available in Ollama. Pull it first (ollama pull {}).",
                    self.model,
                    self.model
                ));
            }
            return Err(anyhow!("Ollama returned HTTP {}: {}", status, body));
        }

        let parsed: EmbedResponse = response
            .json()
            .await
            .context("Invalid embedding response from Ollama")?;
        if parsed.embeddings.len() != texts.len() {
            return Err(anyhow!(
                "Ollama returned {} embeddings for {} inputs",
                parsed.embeddings.len(),
                texts.len()
            ));
        }
        Ok(parsed.embeddings)
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbedder {
    fn model_id(&self) -> &str {
        &self.model
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(BATCH_SIZE) {
            vectors.extend(self.embed_batch(batch).await?);
        }
        Ok(vectors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Serves one canned HTTP response and returns the raw request it received.
    async fn one_shot_server(status: &str, body: &str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let mut request = String::new();
            // Read until the JSON body has arrived
            while !request.contains("\"input\"") {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                request.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            request
        });
        (format!("http://{addr}/"), handle)
    }

    #[tokio::test]
    async fn ollama_embedder_posts_to_api_embed() {
        let (endpoint, server) = one_shot_server("200 OK", r#"{"embeddings":[[0.1,0.2],[0.3,0.4]]}"#).await;
        let embedder = OllamaEmbedder::new(Some(&endpoint), "bge-m3");
        let vectors = embedder.embed(&["a".into(), "b".into()]).await.unwrap();
        assert_eq!(vectors, vec![vec![0.1, 0.2], vec![0.3, 0.4]]);

        let request = server.await.unwrap();
        assert!(request.starts_with("POST /api/embed "));
        assert!(request.contains(r#""model":"bge-m3""#));
        assert!(request.contains(r#""input":["a","b"]"#));
    }

    #[tokio::test]
    async fn ollama_embedder_reports_missing_model() {
        let (endpoint, _server) =
            one_shot_server("404 Not Found", r#"{"error":"model \"bge-m3\" not found"}"#).await;
        let embedder = OllamaEmbedder::new(Some(&endpoint), "bge-m3");
        let error = embedder.embed(&["a".into()]).await.unwrap_err().to_string();
        assert!(error.contains("ollama pull bge-m3"), "{error}");
    }

    #[tokio::test]
    async fn ollama_embedder_rejects_count_mismatch() {
        let (endpoint, _server) = one_shot_server("200 OK", r#"{"embeddings":[[0.1]]}"#).await;
        let embedder = OllamaEmbedder::new(Some(&endpoint), "m");
        assert!(embedder.embed(&["a".into(), "b".into()]).await.is_err());
    }
}

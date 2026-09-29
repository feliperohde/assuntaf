//! Transcription on a remote server with the OpenAI audio API
//! (`POST <base>/audio/transcriptions`): faster-whisper-server / Speaches,
//! LocalAI, a GPU box on the LAN, Groq or OpenAI. Lets machines with little RAM
//! record without running Whisper/Parakeet locally.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::time::Duration;

use super::provider::{TranscriptResult, TranscriptionError, TranscriptionProvider};

pub const PROVIDER: &str = "remote";
const SAMPLE_RATE: u32 = 16_000;
/// Shorter audio is not sent (Whisper servers reject or hallucinate on it).
const MIN_SAMPLES: usize = 1_600;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTranscriptionConfig {
    /// Base URL of the OpenAI-compatible API, e.g. http://192.168.3.16:8000/v1
    pub endpoint: String,
    pub api_key: Option<String>,
    /// e.g. Systran/faster-whisper-large-v3, whisper-large-v3-turbo (Groq), whisper-1 (OpenAI)
    pub model: String,
}

/// "192.168.3.16:8000" → "http://192.168.3.16:8000/v1"; keeps an explicit path.
pub fn normalize_endpoint(input: &str) -> String {
    let raw = input.trim().trim_end_matches('/');
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("http://{raw}") };
    match url::Url::parse(&with_scheme) {
        Ok(url) if url.path() == "/" || url.path().is_empty() => format!("{}/v1", with_scheme.trim_end_matches('/')),
        _ => with_scheme,
    }
}

pub async fn load_config(pool: &SqlitePool) -> Result<Option<RemoteTranscriptionConfig>, sqlx::Error> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT endpoint, api_key, model FROM remote_transcription WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(endpoint, api_key, model)| {
        let endpoint = endpoint.filter(|e| !e.trim().is_empty())?;
        Some(RemoteTranscriptionConfig {
            endpoint,
            api_key: api_key.filter(|k| !k.trim().is_empty()),
            model: model.unwrap_or_default(),
        })
    }))
}

/// Saves the server and makes it the transcription provider.
pub async fn save_config(pool: &SqlitePool, config: &RemoteTranscriptionConfig) -> Result<RemoteTranscriptionConfig, sqlx::Error> {
    let normalized = RemoteTranscriptionConfig {
        endpoint: normalize_endpoint(&config.endpoint),
        api_key: config.api_key.as_deref().map(str::trim).filter(|k| !k.is_empty()).map(String::from),
        model: config.model.trim().to_string(),
    };
    sqlx::query(
        "INSERT INTO remote_transcription (id, endpoint, api_key, model) VALUES (1, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET endpoint = excluded.endpoint, api_key = excluded.api_key, model = excluded.model",
    )
    .bind(&normalized.endpoint)
    .bind(&normalized.api_key)
    .bind(&normalized.model)
    .execute(pool)
    .await?;
    crate::database::repositories::setting::SettingsRepository::save_transcript_config(pool, PROVIDER, &normalized.model)
        .await?;
    Ok(normalized)
}

/// 16-bit PCM mono WAV.
pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        wav.extend_from_slice(&value.to_le_bytes());
    }
    wav
}

fn root_cause(e: &reqwest::Error) -> String {
    let mut source: &dyn std::error::Error = e;
    while let Some(next) = source.source() {
        source = next;
    }
    source.to_string()
}

pub struct RemoteTranscriptionProvider {
    client: reqwest::Client,
    config: RemoteTranscriptionConfig,
}

impl RemoteTranscriptionProvider {
    pub fn new(config: RemoteTranscriptionConfig) -> Self {
        Self { client: crate::net::client_for(&config.endpoint), config }
    }

    /// Sends one WAV to the server and returns the text.
    pub async fn transcribe_wav(&self, wav: Vec<u8>, language: Option<&str>) -> Result<String, String> {
        let file = reqwest::multipart::Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| e.to_string())?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", file)
            .text("model", self.config.model.clone())
            .text("response_format", "json")
            .text("temperature", "0");
        // "auto" / "auto-translate" mean: let the server detect
        if let Some(language) = language.filter(|l| !l.is_empty() && !l.starts_with("auto")) {
            form = form.text("language", language.to_string());
        }
        let mut request = self
            .client
            .post(format!("{}/audio/transcriptions", self.config.endpoint.trim_end_matches('/')))
            .timeout(Duration::from_secs(120))
            .multipart(form);
        if let Some(key) = &self.config.api_key {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .await
            .map_err(|e| format!("Cannot reach the transcription server at {}: {}", self.config.endpoint, root_cause(&e)))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("Transcription server returned HTTP {}: {}", status.as_u16(), body.chars().take(300).collect::<String>()));
        }
        #[derive(Deserialize)]
        struct Reply {
            text: String,
        }
        serde_json::from_str::<Reply>(&body)
            .map(|r| r.text.trim().to_string())
            .map_err(|_| format!("Unexpected answer from the transcription server: {}", body.chars().take(200).collect::<String>()))
    }

    /// Checks URL, key and model with one second of silence.
    pub async fn test(&self) -> Result<(), String> {
        self.transcribe_wav(encode_wav(&vec![0.0; SAMPLE_RATE as usize], SAMPLE_RATE), None)
            .await
            .map(|_| ())
    }
}

#[async_trait]
impl TranscriptionProvider for RemoteTranscriptionProvider {
    async fn transcribe(&self, audio: Vec<f32>, language: Option<String>) -> Result<TranscriptResult, TranscriptionError> {
        if audio.len() < MIN_SAMPLES {
            return Err(TranscriptionError::AudioTooShort { samples: audio.len(), minimum: MIN_SAMPLES });
        }
        let text = self
            .transcribe_wav(encode_wav(&audio, SAMPLE_RATE), language.as_deref())
            .await
            .map_err(TranscriptionError::EngineFailed)?;
        Ok(TranscriptResult { text, confidence: None, is_partial: false })
    }

    async fn is_model_loaded(&self) -> bool {
        true
    }

    async fn get_current_model(&self) -> Option<String> {
        Some(self.config.model.clone())
    }

    fn provider_name(&self) -> &'static str {
        "Remote server (OpenAI-compatible)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn endpoints_get_scheme_and_v1() {
        assert_eq!(normalize_endpoint("192.168.3.16:8000"), "http://192.168.3.16:8000/v1");
        assert_eq!(normalize_endpoint("http://gpu.lan:8000/"), "http://gpu.lan:8000/v1");
        assert_eq!(normalize_endpoint("https://api.groq.com/openai/v1"), "https://api.groq.com/openai/v1");
    }

    #[test]
    fn wav_header_is_valid() {
        let wav = encode_wav(&[0.0, 0.5, -0.5, 1.0], 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 8);
        assert_eq!(wav.len(), 44 + 8);
        assert_eq!(i16::from_le_bytes([wav[50], wav[51]]), i16::MAX);
    }

    #[tokio::test]
    async fn posts_multipart_audio_and_reads_text() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = vec![0u8; 65536];
            // Read until the multipart body is complete (closing boundary)
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request);
                if text.contains("name=\"temperature\"") && text.trim_end().ends_with("--") {
                    break;
                }
            }
            let body = r#"{"text":" Bom dia, o ABC-123 está pronto. "}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&request).to_string()
        });

        let provider = RemoteTranscriptionProvider::new(RemoteTranscriptionConfig {
            endpoint: normalize_endpoint(&format!("127.0.0.1:{}", addr.port())),
            api_key: Some("secret".into()),
            model: "whisper-large-v3".into(),
        });
        let result = provider.transcribe(vec![0.1; 16_000], Some("pt".into())).await.unwrap();
        assert_eq!(result.text, "Bom dia, o ABC-123 está pronto.");

        let request = server.await.unwrap();
        assert!(request.starts_with("POST /v1/audio/transcriptions "));
        assert!(request.to_lowercase().contains("authorization: bearer secret"));
        assert!(request.contains("name=\"model\"\r\n\r\nwhisper-large-v3"));
        assert!(request.contains("name=\"language\"\r\n\r\npt"));
        assert!(request.contains("filename=\"audio.wav\""));

        assert!(matches!(
            provider.transcribe(vec![0.1; 100], None).await,
            Err(TranscriptionError::AudioTooShort { .. })
        ));
    }

    #[tokio::test]
    async fn server_errors_are_reported() {
        let port = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
        let provider = RemoteTranscriptionProvider::new(RemoteTranscriptionConfig {
            endpoint: normalize_endpoint(&format!("127.0.0.1:{port}")),
            api_key: None,
            model: "m".into(),
        });
        let error = provider.test().await.unwrap_err();
        assert!(error.contains("Cannot reach the transcription server"), "{error}");
    }
}

// ============================================================================
// Tauri commands
// ============================================================================

pub mod commands {
    use super::*;
    use crate::state::AppState;

    #[tauri::command]
    pub async fn get_remote_transcription_config(
        state: tauri::State<'_, AppState>,
    ) -> Result<Option<RemoteTranscriptionConfig>, String> {
        load_config(state.db_manager.pool()).await.map_err(|e| e.to_string())
    }

    /// Saves the server and switches transcription to it.
    #[tauri::command]
    pub async fn save_remote_transcription_config(
        state: tauri::State<'_, AppState>,
        config: RemoteTranscriptionConfig,
    ) -> Result<RemoteTranscriptionConfig, String> {
        if config.endpoint.trim().is_empty() || config.model.trim().is_empty() {
            return Err("Server URL and model are required".to_string());
        }
        save_config(state.db_manager.pool(), &config).await.map_err(|e| e.to_string())
    }

    /// Transcribes one second of silence to check URL, key and model.
    #[tauri::command]
    pub async fn test_remote_transcription(config: RemoteTranscriptionConfig) -> Result<String, String> {
        let config = RemoteTranscriptionConfig { endpoint: normalize_endpoint(&config.endpoint), ..config };
        let endpoint = config.endpoint.clone();
        RemoteTranscriptionProvider::new(config).test().await?;
        Ok(endpoint)
    }
}

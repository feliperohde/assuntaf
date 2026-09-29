//! Chat-completion adapter for question answering. Reuses the provider, model and
//! credentials configured for summaries, so there is one place to set up the LLM.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use sqlx::SqlitePool;
use std::path::PathBuf;

use crate::database::repositories::setting::SettingsRepository;
use crate::summary::llm_client::{generate_summary, LLMProvider};

#[async_trait]
pub trait ChatModel: Send + Sync {
    async fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String>;
}

/// The summary LLM configuration, resolved once per request.
pub struct ConfiguredChatModel {
    client: Client,
    provider: LLMProvider,
    model: String,
    api_key: String,
    ollama_endpoint: Option<String>,
    custom_openai_endpoint: Option<String>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    app_data_dir: Option<PathBuf>,
}

impl ConfiguredChatModel {
    pub async fn from_settings(pool: &SqlitePool, app_data_dir: Option<PathBuf>) -> Result<Self> {
        let settings = SettingsRepository::get_model_config(pool)
            .await?
            .ok_or_else(|| anyhow!("No summary model configured. Choose one in Settings → Summary."))?;
        let provider = LLMProvider::from_str(&settings.provider).map_err(|e| anyhow!(e))?;

        let mut model = ConfiguredChatModel {
            client: Client::new(),
            provider: provider.clone(),
            model: settings.model.clone(),
            api_key: String::new(),
            ollama_endpoint: None,
            custom_openai_endpoint: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            app_data_dir,
        };

        match provider {
            LLMProvider::Ollama => model.ollama_endpoint = settings.ollama_endpoint.clone(),
            LLMProvider::BuiltInAI => {}
            LLMProvider::CustomOpenAI => {
                let config = SettingsRepository::get_custom_openai_config(pool)
                    .await?
                    .ok_or_else(|| anyhow!("Custom OpenAI provider selected but not configured"))?;
                model.custom_openai_endpoint = Some(config.endpoint);
                model.api_key = config.api_key.unwrap_or_default();
                model.max_tokens = config.max_tokens.map(|t| t as u32);
                model.temperature = config.temperature;
                model.top_p = config.top_p;
            }
            _ => {
                model.api_key = SettingsRepository::get_api_key(pool, &settings.provider)
                    .await?
                    .filter(|k| !k.is_empty())
                    .ok_or_else(|| anyhow!("API key not found for {}", settings.provider))?;
            }
        }
        Ok(model)
    }
}

#[async_trait]
impl ChatModel for ConfiguredChatModel {
    async fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        let completion = generate_summary(
            &self.client,
            &self.provider,
            &self.model,
            &self.api_key,
            system_prompt,
            user_prompt,
            self.ollama_endpoint.as_deref(),
            self.custom_openai_endpoint.as_deref(),
            self.max_tokens,
            self.temperature,
            self.top_p,
            self.app_data_dir.as_ref(),
            None,
        )
        .await
        .map_err(|e| anyhow!(e))?;
        Ok(completion.content)
    }
}

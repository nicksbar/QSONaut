use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationContext {
    pub band: String,
    pub frequency_hz: u64,
    pub noise_floor_db: f32,
    pub ft8_decodes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub content: String,
}

#[async_trait]
pub trait LanguageModel: Send + Sync {
    async fn chat(&self, context: &StationContext, messages: &[Message]) -> Result<Response>;
}

#[derive(Debug, Default)]
pub struct NullLanguageModel;

#[async_trait]
impl LanguageModel for NullLanguageModel {
    async fn chat(&self, _context: &StationContext, _messages: &[Message]) -> Result<Response> {
        Ok(Response {
            content: "AI disabled (NullLanguageModel)".to_string(),
        })
    }
}

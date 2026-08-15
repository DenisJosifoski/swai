/// Ollama `/api/generate` request.
#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
pub struct OllamaGenerateRequest {
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub options: Option<OllamaOptions>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub images: Option<Vec<String>>,
}

/// Ollama `/api/chat` request.
#[derive(Debug, serde::Deserialize)]
pub struct OllamaChatRequest {
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub options: Option<OllamaOptions>,
}

/// Ollama message role/content.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct OllamaMessage {
    pub role: String,
    pub content: serde_json::Value,
}

/// Ollama generation options (subset we care about).
#[derive(Debug, serde::Deserialize)]
pub struct OllamaOptions {
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub num_predict: Option<i64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub top_k: Option<i64>,
}

/// Ollama streaming chunk for `/api/generate`.
#[derive(Debug, serde::Serialize)]
pub struct OllamaGenerateChunk {
    pub model: String,
    #[serde(rename = "created_at")]
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<OllamaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_duration: Option<u64>,
}

/// Ollama streaming chunk for `/api/chat`.
#[derive(Debug, serde::Serialize)]
pub struct OllamaChatChunk {
    pub model: String,
    #[serde(rename = "created_at")]
    pub created_at: String,
    pub message: OllamaMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_duration: Option<u64>,
}

/// Ollama non-streaming response for `/api/generate`.
#[derive(Debug, serde::Serialize)]
pub struct OllamaGenerateResponse {
    pub model: String,
    #[serde(rename = "created_at")]
    pub created_at: String,
    pub message: OllamaMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_duration: Option<u64>,
}

/// Ollama non-streaming response for `/api/chat`.
#[derive(Debug, serde::Serialize)]
pub struct OllamaChatResponse {
    pub model: String,
    #[serde(rename = "created_at")]
    pub created_at: String,
    pub message: OllamaMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_duration: Option<u64>,
}

/// Ollama `/api/tags` response.
#[derive(Debug, serde::Serialize)]
#[allow(dead_code)]
pub struct OllamaTagsResponse {
    pub models: Vec<OllamaTagEntry>,
}

/// Single entry in the `/api/tags` model list.
#[derive(Debug, serde::Serialize)]
pub struct OllamaTagEntry {
    pub name: String,
    #[serde(rename = "model")]
    pub model_id: String,
    pub modified_at: String,
    pub size: u64,
}

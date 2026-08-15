use std::io::{Read, Result as IoResult};
use super::ollama_types::*;

/// Streaming body that transforms OpenAI SSE chunks into Ollama NDJSON.
pub struct OllamaStreamingBody {
    pub chunks: Vec<Vec<u8>>,
    pub pos: usize,
}

impl Read for OllamaStreamingBody {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if self.pos >= self.chunks.len() {
            return Ok(0);
        }

        let chunk = &self.chunks[self.pos];
        self.pos += 1;

        let to_copy = std::cmp::min(buf.len(), chunk.len());
        buf[..to_copy].copy_from_slice(&chunk[..to_copy]);
        if chunk.len() > to_copy {
            let remainder = chunk[to_copy..].to_vec();
            self.chunks.insert(self.pos, remainder);
        }
        Ok(to_copy)
    }
}

/// Streaming body that transforms OpenAI SSE chunks into Ollama chat NDJSON.
pub struct OllamaChatStreamingBody {
    pub chunks: Vec<Vec<u8>>,
    pub pos: usize,
}

impl Read for OllamaChatStreamingBody {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if self.pos >= self.chunks.len() {
            return Ok(0);
        }

        let chunk = &self.chunks[self.pos];
        self.pos += 1;

        let to_copy = std::cmp::min(buf.len(), chunk.len());
        buf[..to_copy].copy_from_slice(&chunk[..to_copy]);
        if chunk.len() > to_copy {
            let remainder = chunk[to_copy..].to_vec();
            self.chunks.insert(self.pos, remainder);
        }
        Ok(to_copy)
    }
}

/// Helper to build Ollama streaming chunks from an OpenAI SSE response body.
pub fn build_ollama_generate_chunks(
    model: &str,
    body_str: &str,
) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    for line in body_str.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            let data = data.trim();
            if data == "[DONE]" {
                let final_chunk = OllamaGenerateChunk {
                    model: model.to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    message: None,
                    done: Some(true),
                    total_duration: None,
                    eval_count: None,
                    eval_duration: None,
                };
                let json = serde_json::to_string(&final_chunk).unwrap_or_default();
                let evt = format!("data: {}\n\n", json);
                chunks.push(evt.into_bytes());
            } else if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(choices) = json_val.get("choices").and_then(|c| c.as_array()) {
                    if let Some(first) = choices.first() {
                        if let Some(delta) = first.get("delta") {
                            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                if !content.is_empty() {
                                    let chunk = OllamaGenerateChunk {
                                        model: model.to_string(),
                                        created_at: chrono::Utc::now().to_rfc3339(),
                                        message: Some(OllamaMessage {
                                            role: "assistant".to_string(),
                                            content: serde_json::Value::String(content.to_string()),
                                        }),
                                        done: None,
                                        total_duration: None,
                                        eval_count: None,
                                        eval_duration: None,
                                    };
                                    let json = serde_json::to_string(&chunk).unwrap_or_default();
                                    let evt = format!("data: {}\n\n", json);
                                    chunks.push(evt.into_bytes());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    chunks
}

/// Helper to build Ollama chat streaming chunks from an OpenAI SSE response body.
pub fn build_ollama_chat_chunks(
    model: &str,
    body_str: &str,
) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    for line in body_str.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            let data = data.trim();
            if data == "[DONE]" {
                let final_chunk = OllamaChatChunk {
                    model: model.to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    message: OllamaMessage {
                        role: "assistant".to_string(),
                        content: serde_json::Value::String(String::new()),
                    },
                    done: Some(true),
                    total_duration: None,
                    eval_count: None,
                    eval_duration: None,
                };
                let json = serde_json::to_string(&final_chunk).unwrap_or_default();
                let evt = format!("data: {}\n\n", json);
                chunks.push(evt.into_bytes());
            } else if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(choices) = json_val.get("choices").and_then(|c| c.as_array()) {
                    if let Some(first) = choices.first() {
                        if let Some(delta) = first.get("delta") {
                            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                let chunk = OllamaChatChunk {
                                    model: model.to_string(),
                                    created_at: chrono::Utc::now().to_rfc3339(),
                                    message: OllamaMessage {
                                        role: "assistant".to_string(),
                                        content: serde_json::Value::String(content.to_string()),
                                    },
                                    done: None,
                                    total_duration: None,
                                    eval_count: None,
                                    eval_duration: None,
                                };
                                let json = serde_json::to_string(&chunk).unwrap_or_default();
                                let evt = format!("data: {}\n\n", json);
                                chunks.push(evt.into_bytes());
                            }
                        }
                    }
                }
            }
        }
    }
    chunks
}

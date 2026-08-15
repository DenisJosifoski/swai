use std::sync::{Arc, Mutex};
use tiny_http::{Header, Request, Response};

pub use super::ollama_chat::{handle_ollama_chat, ollama_chat_to_openai};
pub use super::ollama_generate::{handle_ollama_generate, ollama_generate_to_openai};
pub use super::ollama_streaming::*;
pub use super::ollama_types::*;
use super::router::error_response;
use super::state::ProxyState;

/// Check whether the request path is one of the Ollama endpoints we translate.
pub fn is_ollama_endpoint(path: &str) -> bool {
    path == "/api/generate" || path == "/api/chat" || path == "/api/tags"
}

/// Build an Ollama `/api/tags` response from the proxy state.
pub fn build_ollama_tags_from_state(proxy_state: &ProxyState) -> serde_json::Value {
    let mut models = Vec::new();
    for model_id in proxy_state.active_models.keys() {
        models.push(OllamaTagEntry {
            name: model_id.clone(),
            model_id: model_id.clone(),
            modified_at: chrono::Utc::now().to_rfc3339(),
            size: 0,
        });
    }

    if models.is_empty() {
        models.push(OllamaTagEntry {
            name: "swai-model".to_string(),
            model_id: "swai-model".to_string(),
            modified_at: chrono::Utc::now().to_rfc3339(),
            size: 0,
        });
    }

    serde_json::json!({ "models": models })
}

/// Handle Ollama `/api/tags` — return a dynamic model list from active models.
pub fn handle_ollama_tags(req: Request, state: &Arc<Mutex<ProxyState>>) {
    let proxy_state = match state.lock() {
        Ok(s) => s,
        Err(_) => {
            let _ = req.respond(error_response(500, "Internal proxy error"));
            return;
        }
    };

    let tags = build_ollama_tags_from_state(&proxy_state);
    let body = serde_json::to_vec(&tags).unwrap_or_default();

    let headers = vec![
        Header::from_bytes("content-type", b"application/json").unwrap(),
        Header::from_bytes("content-length", body.len().to_string().as_bytes()).unwrap(),
        Header::from_bytes("access-control-allow-origin", b"*").unwrap(),
    ];

    let response = Response::new(
        tiny_http::StatusCode(200),
        headers,
        std::io::Cursor::new(body),
        None,
        None,
    );

    let _ = req.respond(response);
}

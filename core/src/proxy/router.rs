use std::sync::{Arc, Mutex};
use tiny_http::{Header, Request, Response};
use tracing::{debug, error};

use super::anthropic::process_anthropic_payload;
use super::ollama::{handle_ollama_chat, handle_ollama_generate, handle_ollama_tags, is_ollama_endpoint};
use super::openai::{handle_v1_models, normalize_codex_payload, responses_adapter};
use super::state::ProxyState;
use super::streaming::{translate_openai_sse_to_responses, ResponsesStreamingBody, StreamingBody};

/// Handle an incoming proxy request by inspecting state and forwarding.
pub fn handle_proxy_request(
    mut req: Request,
    state: Arc<Mutex<ProxyState>>,
    client: reqwest::blocking::Client,
) {
    let proxy_state = match state.lock() {
        Ok(s) => s,
        Err(e) => {
            error!("proxy state lock poisoned: {}", e);
            return;
        }
    };

    // No active model → 503 Service Unavailable
    if proxy_state.primary_port.is_none() && proxy_state.active_models.is_empty() {
        drop(proxy_state);
        let _ = req.respond(error_response(
            503,
            "No active model server in SWAI",
        ));
        return;
    }

    // Model is currently starting / restarting → 503
    if proxy_state.is_loading {
        drop(proxy_state);
        let _ = req.respond(error_response(
            503,
            "Model server is currently starting/restarting",
        ));
        return;
    }

    // Read the request body to inspect for a `model` field.
    let mut request_body = Vec::new();
    let reader = req.as_reader();
    let _ = reader.read_to_end(&mut request_body);

    // Try to resolve target port from the `model` field in the request payload.
    let target_port = resolve_target_port(&proxy_state, &request_body);
    drop(proxy_state);

    let target_port = match target_port {
        Some(port) => port,
        None => {
            // No matching model found — fall back to primary.
            let primary = match state.lock() {
                Ok(s) => s.primary_port,
                Err(_) => None,
            };
            match primary {
                Some(p) => p,
                None => {
                    let _ = req.respond(error_response(
                        503,
                        "No active model server in SWAI",
                    ));
                    return;
                }
            }
        }
    };

    // Handle OpenAI /v1/models endpoint to list all currently running models.
    let path_and_query = req.url().to_string();
    if req.method().as_str() == "GET" && (path_and_query == "/v1/models" || path_and_query == "/models") {
        handle_v1_models(req, &state);
        return;
    }

    // Ollama endpoint translation — handle before generic forwarding.
    if is_ollama_endpoint(&path_and_query) {
        match path_and_query.as_str() {
            "/api/tags" => {
                handle_ollama_tags(req, &state);
                return;
            }
            "/api/generate" => {
                handle_ollama_generate(req, state, client, target_port);
                return;
            }
            "/api/chat" => {
                handle_ollama_chat(req, state, client, target_port);
                return;
            }
            _ => {}
        }
    }

    // Build headers for the forwarded request, stripping hop-by-hop headers
    let mut forward_headers = Vec::new();
    for header in req.headers() {
        let field_name = header.field.as_str();
        if is_hop_by_hop_header(field_name.as_ref()) {
            continue;
        }
        let field_bytes = field_name.as_bytes();
        forward_headers.push(
            Header::from_bytes(field_bytes, header.value.as_bytes())
                .unwrap_or_else(|_| {
                    Header::from_bytes(field_bytes, b"")
                        .expect("header construction should never fail")
                }),
        );
    }

    let is_responses = path_and_query.contains("/v1/responses");

    if is_responses {
        let active_model_id = "swai-active-model";
        match responses_adapter(&request_body, active_model_id) {
            Ok(translated) => {
                request_body = match serde_json::to_vec(&translated) {
                    Ok(b) => b,
                    Err(e) => {
                        debug!("failed to serialize translated responses body: {}", e);
                        let _ = req.respond(error_response(500, "Failed to translate request"));
                        return;
                    }
                };
            }
            Err(e) => {
                debug!("responses adapter failed: {}", e);
                let _ = req.respond(error_response(400, &e));
                return;
            }
        }
    }

    // Normalize Codex payloads for non-Responses OpenAI chat endpoints.
    if !request_body.is_empty() && path_and_query.contains("/v1/chat/completions") {
        if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&request_body) {
            normalize_codex_payload(&mut json_val);
            if let Ok(normalized_bytes) = serde_json::to_vec(&json_val) {
                request_body = normalized_bytes;
            }
        }
    }

    // Process Anthropic /v1/messages: auto-compact, checkpoint, and anti-hallucination guard
    if !request_body.is_empty() && path_and_query.contains("/v1/messages") {
        if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&request_body) {
            process_anthropic_payload(&mut json_val, request_body.len(), &state, target_port);
            if let Ok(normalized_bytes) = serde_json::to_vec(&json_val) {
                request_body = normalized_bytes;
            }
        }
    }

    let effective_path = if is_responses {
        path_and_query.replacen("/v1/responses", "/v1/chat/completions", 1)
    } else {
        path_and_query.clone()
    };
    let target_url = format!("http://127.0.0.1:{}{}", target_port, effective_path);

    let method = match req.method().as_str() {
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        "HEAD" => reqwest::Method::HEAD,
        "OPTIONS" => reqwest::Method::OPTIONS,
        _ => reqwest::Method::GET,
    };

    let mut request_builder = client.request(method, &target_url);

    for header in &forward_headers {
        let field_name = std::str::from_utf8(header.field.as_str().as_bytes()).unwrap_or("");
        if field_name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        request_builder = request_builder.header(field_name, header.value.as_str());
    }

    if !request_body.is_empty() {
        request_builder = request_builder.body(request_body.clone());
    }

    let is_get = req.method().as_str() == "GET";
    let is_models_endpoint = req.url().starts_with("/v1/models");

    if is_models_endpoint && is_get {
        let has_auth = req.headers().iter().any(|h| {
            h.field.as_str() == "authorization"
                || h.field.as_str() == "Authorization"
                || h.field.as_str() == "AUTHORIZATION"
                && h.value.as_str().starts_with("Bearer ")
        });

        if let Some(builder) = request_builder.try_clone() {
            if let Ok(resp) = builder.send() {
                let status = resp.status().as_u16();
                if let Ok(body_bytes) = resp.bytes() {
                    if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                        if let Some(data_arr) = json_val.get_mut("data").and_then(|d| d.as_array_mut()) {
                            let mut extra_items = Vec::new();
                            for item in data_arr.iter_mut() {
                                if let Some(obj) = item.as_object_mut() {
                                    if has_auth {
                                        obj.insert(
                                            "object".to_string(),
                                            serde_json::Value::String("model".to_string()),
                                        );
                                    } else {
                                        let clean_id = if let Some(raw_id) = obj.get("id").and_then(|v| v.as_str()) {
                                            std::path::Path::new(raw_id)
                                                .file_stem()
                                                .and_then(|s| s.to_str())
                                                .unwrap_or("swai-model")
                                                .replace("-Q4_K_M", "")
                                                .replace("-Q4_K_S", "")
                                                .replace("-Q5_K_M", "")
                                                .replace("-Q8_0", "")
                                        } else {
                                            "swai-model".to_string()
                                        };

                                        obj.insert(
                                            "id".to_string(),
                                            serde_json::Value::String(clean_id),
                                        );

                                        let mut spoof_obj = obj.clone();
                                        spoof_obj.insert(
                                            "id".to_string(),
                                            serde_json::Value::String("claude-sonnet-4-5".to_string()),
                                        );
                                        extra_items.push(serde_json::Value::Object(spoof_obj));
                                    }
                                }
                            }
                            data_arr.extend(extra_items);
                        }
                        let modified_bytes = serde_json::to_vec(&json_val).unwrap_or_else(|_| body_bytes.to_vec());
                        let mut response_headers = Vec::new();
                        response_headers.push(Header::from_bytes("content-type", b"application/json").unwrap());
                        response_headers.push(Header::from_bytes("content-length", modified_bytes.len().to_string().as_bytes()).unwrap());
                        let tiny_response = Response::new(
                            tiny_http::StatusCode(status),
                            response_headers,
                            std::io::Cursor::new(modified_bytes),
                            None,
                            None,
                        );
                        let _ = req.respond(tiny_response);
                        return;
                    }
                }
            }
        }
    }

    let response = match request_builder.send() {
        Ok(resp) => resp,
        Err(e) => {
            debug!("forward failed to model server {}: {}", target_port, e);
            let _ = req.respond(error_response(503, "Model server unavailable"));
            return;
        }
    };

    let status = response.status().as_u16();
    let resp_headers = response.headers();

    let mut response_headers = Vec::new();
    for (name, value) in resp_headers.iter() {
        if is_hop_by_hop_header(name.as_str()) {
            continue;
        }
        response_headers.push(
            Header::from_bytes(name.as_str(), value.as_bytes())
                .unwrap_or_else(|_| {
                    Header::from_bytes(name.as_str(), b"")
                        .expect("header construction should never fail")
                }),
        );
    }

    if let Some(content_length) = response.content_length() {
        response_headers.push(
            Header::from_bytes("content-length", content_length.to_string().as_bytes())
                .unwrap_or_else(|_| {
                    Header::from_bytes("content-length", b"0")
                        .expect("header construction should never fail")
                }),
        );
    }

    let is_responses_api = path_and_query.contains("/v1/responses");

    if is_responses_api {
        let body_bytes = match response.bytes() {
            Ok(b) => b,
            Err(e) => {
                debug!("failed to read responses API response body: {}", e);
                let _ = req.respond(error_response(502, "Failed to read model response"));
                return;
            }
        };
        let body_str = String::from_utf8_lossy(&body_bytes).to_string();
        let translated_events = translate_openai_sse_to_responses(&body_str, "swai-active-model");
        let streaming_body = ResponsesStreamingBody {
            events: translated_events,
            pos: 0,
        };

        let tiny_response = Response::new(
            tiny_http::StatusCode(status),
            response_headers,
            Box::new(streaming_body),
            None,
            None,
        );

        if let Err(e) = req.respond(tiny_response) {
            debug!("failed to respond to responses API client: {}", e);
        }
    } else {
        let streaming_body = StreamingBody {
            reader: response,
            is_responses_api: false,
            sent_completion: false,
            leftover: Vec::new(),
        };

        let tiny_response = Response::new(
            tiny_http::StatusCode(status),
            response_headers,
            Box::new(streaming_body),
            None,
            None,
        );

        if let Err(e) = req.respond(tiny_response) {
            debug!("failed to respond to proxy client: {}", e);
        }
    }
}

/// Resolve the target port for an incoming request by inspecting its JSON body.
pub fn resolve_target_port(state: &ProxyState, body: &[u8]) -> Option<u16> {
    if body.is_empty() {
        return None;
    }

    let has_model_key = body.windows(7).any(|w| {
        w.eq_ignore_ascii_case(b"\"model\"")
            || w.eq_ignore_ascii_case(b"'model'")
    });
    if !has_model_key {
        return None;
    }

    let json_val = match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(v) => v,
        Err(_) => return None,
    };

    let model_id = json_val
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("");

    if model_id.is_empty() {
        return None;
    }

    for (id, &port) in &state.active_models {
        if id == model_id {
            return Some(port);
        }
    }

    None
}

/// Check if a header name is a hop-by-hop header per RFC 7230 §6.1.
pub fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade",
    )
}

/// Build an error response with a JSON body.
pub fn error_response(status: u16, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = format!("{{\"error\": \"{}\"}}", message);
    Response::from_data(body.into_bytes())
        .with_status_code(tiny_http::StatusCode(status))
        .with_header(
            Header::from_bytes("content-type", b"application/json")
                .unwrap_or_else(|_| {
                    Header::from_bytes("content-type", b"application/json")
                        .expect("should never fail")
                }),
        )
}

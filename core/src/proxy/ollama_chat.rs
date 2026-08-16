use std::sync::{Arc, Mutex};
use tiny_http::{Header, Request, Response};
use tracing::{debug, error};

use super::ollama_streaming::{build_ollama_chat_chunks, OllamaChatStreamingBody};
use super::ollama_types::*;
use super::router::{error_response, is_hop_by_hop_header};
use super::state::ProxyState;

/// Convert an Ollama chat request into an OpenAI chat-completions payload.
pub fn ollama_chat_to_openai(req: &OllamaChatRequest) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            let content = match &m.content {
                serde_json::Value::String(s) => s.clone(),
                _ => serde_json::to_string(&m.content).unwrap_or_default(),
            };
            serde_json::json!({
                "role": m.role,
                "content": content,
            })
        })
        .collect();

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
        "stream": req.stream,
    });

    if let Some(ref opts) = req.options {
        if let Some(temp) = opts.temperature {
            body["temperature"] = serde_json::Value::Number(
                serde_json::Number::from_f64(temp).unwrap_or_else(|| serde_json::Number::from(0)),
            );
        }
        if let Some(n) = opts.num_predict {
            body["max_tokens"] = serde_json::Value::Number(serde_json::Number::from(n));
        }
        if let Some(tp) = opts.top_p {
            body["top_p"] = serde_json::Value::Number(
                serde_json::Number::from_f64(tp).unwrap_or_else(|| serde_json::Number::from(0)),
            );
        }
        if let Some(tk) = opts.top_k {
            body["top_k"] = serde_json::Value::Number(serde_json::Number::from(tk));
        }
    }

    body
}

/// Handle Ollama `/api/chat` — translate to OpenAI and forward.
pub fn handle_ollama_chat(
    mut req: Request,
    state: Arc<Mutex<ProxyState>>,
    client: reqwest::blocking::Client,
    target_port: u16,
) {
    let proxy_state = match state.lock() {
        Ok(s) => s,
        Err(e) => {
            error!("proxy state lock poisoned: {}", e);
            return;
        }
    };

    if proxy_state.active_models.is_empty() {
        drop(proxy_state);
        let _ = req.respond(error_response(503, "No active model server in SWAI"));
        return;
    }

    if proxy_state.is_loading {
        drop(proxy_state);
        let _ = req.respond(error_response(
            503,
            "Model server is currently starting/restarting",
        ));
        return;
    }

    drop(proxy_state);

    let mut request_body = Vec::new();
    let reader = req.as_reader();
    if let Err(e) = reader.read_to_end(&mut request_body) {
        debug!("failed to read ollama chat request body: {}", e);
        let _ = req.respond(error_response(500, "Failed to read request body"));
        return;
    }

    let ollama_req: OllamaChatRequest = match serde_json::from_slice(&request_body) {
        Ok(r) => r,
        Err(e) => {
            debug!("failed to parse ollama chat request: {}", e);
            let _ = req.respond(error_response(
                400,
                &format!("Invalid Ollama request: {}", e),
            ));
            return;
        }
    };

    let openai_body = ollama_chat_to_openai(&ollama_req);
    let openai_bytes = match serde_json::to_vec(&openai_body) {
        Ok(b) => b,
        Err(e) => {
            debug!("failed to serialize openai body: {}", e);
            let _ = req.respond(error_response(500, "Failed to serialize request"));
            return;
        }
    };

    let target_url = format!("http://127.0.0.1:{}/v1/chat/completions", target_port);
    let mut forward_headers = Vec::new();
    for header in req.headers() {
        let field_name = header.field.as_str();
        if is_hop_by_hop_header(field_name.as_ref()) {
            continue;
        }
        let field_bytes = field_name.as_bytes();
        forward_headers.push(
            Header::from_bytes(field_bytes, header.value.as_bytes()).unwrap_or_else(|_| {
                Header::from_bytes(field_bytes, b"").expect("header construction should never fail")
            }),
        );
    }

    let mut request_builder = client
        .post(&target_url)
        .header("content-type", "application/json")
        .body(openai_bytes);

    for header in &forward_headers {
        let field_name = std::str::from_utf8(header.field.as_str().as_bytes()).unwrap_or("");
        if field_name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        request_builder = request_builder.header(field_name, header.value.as_str());
    }

    let response = match request_builder.send() {
        Ok(resp) => resp,
        Err(e) => {
            debug!("ollama chat forward failed: {}", e);
            let _ = req.respond(error_response(503, "Model server unavailable"));
            return;
        }
    };

    let status = response.status().as_u16();
    let resp_headers = response.headers().clone();

    let mut response_headers = Vec::new();
    for (name, value) in resp_headers.iter() {
        if is_hop_by_hop_header(name.as_str()) {
            continue;
        }
        response_headers.push(
            Header::from_bytes(name.as_str(), value.as_bytes()).unwrap_or_else(|_| {
                Header::from_bytes(name.as_str(), b"")
                    .expect("header construction should never fail")
            }),
        );
    }

    if ollama_req.stream {
        let body_bytes = match response.bytes() {
            Ok(b) => b,
            Err(e) => {
                debug!("failed to read ollama chat response body: {}", e);
                let _ = req.respond(error_response(502, "Failed to read model response"));
                return;
            }
        };
        let body_str = String::from_utf8_lossy(&body_bytes).to_string();
        let chunks = build_ollama_chat_chunks(&ollama_req.model, &body_str);
        let streaming_body = OllamaChatStreamingBody { chunks, pos: 0 };

        let tiny_response = Response::new(
            tiny_http::StatusCode(status),
            response_headers,
            Box::new(streaming_body),
            None,
            None,
        );

        if let Err(e) = req.respond(tiny_response) {
            debug!("failed to respond to ollama chat client: {}", e);
        }
    } else {
        let content_length = response.content_length();
        match response.bytes() {
            Ok(body_bytes) => {
                if let Ok(json_val) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                    if let Some(choices) = json_val.get("choices").and_then(|c| c.as_array()) {
                        if let Some(first) = choices.first() {
                            if let Some(msg) = first.get("message") {
                                let role = msg
                                    .get("role")
                                    .and_then(|r| r.as_str())
                                    .unwrap_or("assistant");
                                let content =
                                    msg.get("content").and_then(|c| c.as_str()).unwrap_or("");

                                let ollama_resp = OllamaChatResponse {
                                    model: ollama_req.model,
                                    created_at: chrono::Utc::now().to_rfc3339(),
                                    message: OllamaMessage {
                                        role: role.to_string(),
                                        content: serde_json::Value::String(content.to_string()),
                                    },
                                    done: Some(true),
                                    total_duration: None,
                                    eval_count: None,
                                    eval_duration: None,
                                };

                                let resp_bytes = serde_json::to_vec(&ollama_resp)
                                    .unwrap_or_else(|_| body_bytes.to_vec());

                                response_headers.push(
                                    Header::from_bytes("content-type", b"application/json")
                                        .unwrap(),
                                );
                                response_headers.push(
                                    Header::from_bytes(
                                        "content-length",
                                        resp_bytes.len().to_string().as_bytes(),
                                    )
                                    .unwrap(),
                                );

                                let tiny_response = Response::new(
                                    tiny_http::StatusCode(status),
                                    response_headers,
                                    std::io::Cursor::new(resp_bytes),
                                    None,
                                    None,
                                );
                                let _ = req.respond(tiny_response);
                                return;
                            }
                        }
                    }
                }
                let mut resp_headers_with_ct = response_headers.clone();
                resp_headers_with_ct.push(
                    Header::from_bytes("content-type", b"application/json").unwrap_or_else(|_| {
                        Header::from_bytes("content-type", b"application/json")
                            .expect("should never fail")
                    }),
                );
                if let Some(cl) = content_length {
                    resp_headers_with_ct.push(
                        Header::from_bytes("content-length", cl.to_string().as_bytes())
                            .unwrap_or_else(|_| {
                                Header::from_bytes("content-length", b"0")
                                    .expect("should never fail")
                            }),
                    );
                }
                let tiny_response = Response::new(
                    tiny_http::StatusCode(status),
                    resp_headers_with_ct,
                    std::io::Cursor::new(body_bytes.to_vec()),
                    None,
                    None,
                );
                let _ = req.respond(tiny_response);
            }
            Err(e) => {
                debug!("failed to read ollama chat response: {}", e);
                let _ = req.respond(error_response(502, "Failed to read model response"));
            }
        }
    }
}

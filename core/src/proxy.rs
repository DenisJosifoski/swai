//! SWAI — Reverse proxy server.
//!
//! A transparent local HTTP reverse proxy listening on `127.0.0.1:proxy_port`
//! (default 9080). It inspects the active model state dynamically and forwards
//! all incoming API requests to whichever model is currently Ready in the
//! ProcessManager.
//!
//! - Model Ready → forward to `http://127.0.0.1:{active_model_port}`
//! - No model / Error → HTTP 503 with JSON error body
//! - Loading (starting/restarting) → HTTP 503 with "currently starting" message
//!
//! Handles Anthropic Messages API passthrough (`POST /v1/messages`, `GET /v1/models`)
//! and OpenAI-compatible endpoints (`POST /v1/chat/completions`, `POST /v1/completions`).
//! SSE streaming events pass through untouched; hop-by-hop headers are stripped
//! per RFC 7230 §6.1 to prevent SSE connection drops.
//!
//! For `GET /v1/models` requests with an `Authorization: Bearer` header (OpenAI clients),
//! the proxy rewrites raw model paths into OpenAI-compatible format (`{"id": "...", "object": "model"}`).
//!
//! Also provides Ollama-compatible endpoint translators:
//! - `POST /api/generate` — raw generation, maps to `/v1/chat/completions`
//! - `POST /api/chat` — chat messages, maps to `/v1/chat/completions`
//! - `GET /api/tags` — lists configured models in Ollama format

use std::io::{Read, Result as IoResult};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Request, Response, Server};
use tracing::{debug, error, info};

/// Shared state between the proxy server and the application.
///
/// Updated by the app whenever a model starts, stops, switches, or restarts.
/// The proxy reads this state on every incoming request to decide where to
/// forward (or whether to return 503).
///
/// In multi-model mode, `active_models` holds all concurrently running models;
/// `primary_port` is the port of the first-started model (fallback target).
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct ProxyState {
    /// The port of the primary (first-started) active model server.
    /// `None` means no model is running.
    pub primary_port: Option<u16>,

    /// All currently running models, keyed by their configured id.
    /// Each entry maps to the port its server is bound to.
    pub active_models: std::collections::HashMap<String, u16>,

    /// Whether any model is currently in a transitional state (starting / restarting).
    /// When `true`, the proxy returns 503 even if ports are set, because
    /// models on those ports are not yet Ready to serve requests.
    pub is_loading: bool,
}


impl ProxyState {
    /// Create a new proxy state with no active model.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the primary target port and mark all models as loaded (Ready).
    pub fn set_target(&mut self, port: u16) {
        self.primary_port = Some(port);
        self.is_loading = false;
    }

    /// Register a running model (id → port mapping) with the proxy state.
    pub fn add_model(&mut self, id: String, port: u16) {
        self.active_models.insert(id, port);
        // First model added becomes the primary.
        if self.primary_port.is_none() {
            self.primary_port = Some(port);
        }
        self.is_loading = false;
    }

    /// Sync the proxy state with the full set of running models from the
    /// ProcessManager. Replaces the entire `active_models` map and recomputes
    /// `primary_port` from the first entry.
    ///
    /// Call this after any model start/stop to keep the proxy state consistent
    /// for dynamic multi-model routing.
    pub fn sync_models(&mut self, models: Vec<(String, u16)>) {
        self.active_models.clear();
        self.primary_port = None;
        for (id, port) in models {
            self.active_models.insert(id.clone(), port);
            if self.primary_port.is_none() {
                self.primary_port = Some(port);
            }
        }
        self.is_loading = false;
    }

    /// Remove a running model from the proxy state by id.
    pub fn remove_model(&mut self, id: &str) -> Option<u16> {
        let port = self.active_models.remove(id);
        // If we removed the primary, shift to the next available model.
        if let Some(p) = port {
            if self.primary_port == Some(p) {
                self.primary_port = self.active_models.values().next().copied();
            }
        }
        port
    }

    /// Look up the port for a running model by id or name.
    ///
    /// Checks both `id` and `name` fields from config. Returns `None` if not found.
    pub fn find_model_port(&self, identifier: &str) -> Option<u16> {
        // Direct id match
        if let Some(&port) = self.active_models.get(identifier) {
            return Some(port);
        }
        // Name match — caller should pass the config to resolve names.
        // This is handled at a higher level by the app.
        None
    }

    /// Mark the proxy as loading (model is starting/restarting).
    pub fn set_loading(&mut self) {
        self.is_loading = true;
    }

    /// Clear all model state and mark as not loading.
    pub fn clear(&mut self) {
        self.active_models.clear();
        self.primary_port = None;
        self.is_loading = false;
    }
}

/// A reverse proxy server that forwards requests to the active model.
///
/// Runs on a background std::thread (not the GTK main loop). The proxy reads
/// the shared `ProxyState` on every request to determine the forwarding target.
///
/// P2-1 FIX: The `reqwest::blocking::Client` is built once during construction
/// and reused for all proxied requests, instead of creating a new client per
/// request. This avoids the overhead of TCP/TLS handshake setup on every call.
pub struct ProxyServer {
    shutdown_flag: Arc<AtomicBool>,
    stop_tx: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    proxy_port: u16,
    /// Reusable HTTP client for forwarding requests to the model server.
    #[allow(dead_code)]
    client: reqwest::blocking::Client,
}

impl ProxyServer {
    /// Create and start the proxy server on the given port with the provided state.
    ///
    /// Returns `Ok(Self)` if the server started successfully, or an error string
    /// if binding failed (e.g., port already in use).
    pub fn new(proxy_port: u16, state: Arc<Mutex<ProxyState>>) -> Result<Self, String> {
        let addr = format!("127.0.0.1:{}", proxy_port);

        let server = Server::http(&addr)
            .map_err(|e| format!("failed to bind proxy server to {}: {}", addr, e))?;

        // P2-1 FIX: Build the reqwest client once and share it across all requests.
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .map_err(|e| format!("failed to build reqwest client for proxy: {}", e))?;

        // Graceful shutdown via oneshot channel
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let shutdown_flag_clone = Arc::clone(&shutdown_flag);
        let state_for_proxy = Arc::clone(&state);
        let client_for_proxy = client.clone();

        std::thread::spawn(move || {
            info!(
                "reverse proxy started on http://127.0.0.1:{}",
                proxy_port
            );

            for req in server.incoming_requests() {
                // Check shutdown signal first
                if stop_rx.try_recv().is_ok() || shutdown_flag_clone.load(Ordering::Relaxed) {
                    break;
                }

                let state = Arc::clone(&state_for_proxy);
                handle_proxy_request(req, state, client_for_proxy.clone());
            }

            info!("reverse proxy stopped");
        });

        Ok(Self {
            shutdown_flag,
            stop_tx: Mutex::new(Some(stop_tx)),
            proxy_port,
            client,
        })
    }

    /// Gracefully shut down the proxy server.
    pub fn stop(&self) {
        self.shutdown_flag.store(true, Ordering::SeqCst);
        if let Some(tx) = self.stop_tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = tx.send(());
        }
        // Ping the proxy port to instantly unblock tiny_http's accept() loop.
        let port = self.proxy_port;
        std::thread::spawn(move || {
            let _ = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_millis(200))
                .build()
                .and_then(|c| c.get(format!("http://127.0.0.1:{}/", port)).send());
        });
    }
}

impl Drop for ProxyServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Handle an incoming proxy request by inspecting state and forwarding.
///
/// All requests (including Anthropic Messages API endpoints like `POST /v1/messages`
/// and `POST /v1/messages/count_tokens`) are forwarded transparently to the active
/// model server. SSE streaming events pass through untouched; hop-by-hop headers
/// are stripped per RFC 7230 §6.1.
///
/// Multi-model routing (Phase 23): if the request JSON body contains a `model`
/// field that matches a running model's id or name, the request is routed to
/// that specific model's port. Otherwise it falls back to the primary active
/// model port.
fn handle_proxy_request(
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

    // Ollama endpoint translation — handle before generic forwarding.
    let path_and_query = req.url().to_string();
    if is_ollama_endpoint(&path_and_query) {
        match path_and_query.as_str() {
            "/api/tags" => {
                handle_ollama_tags(req);
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
    // per RFC 7230 §6.1 to prevent response-framing edge cases.
    let mut forward_headers = Vec::new();
    for header in req.headers() {
        let field_name = header.field.as_str();
        if is_hop_by_hop_header(field_name.as_ref()) {
            continue; // Skip hop-by-hop headers — they are connection-scoped only.
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

    // Handle Responses API requests specially: translate input → messages,
    // remap model ID, then forward to /v1/chat/completions on the model server.
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

    // Remap model field for POST /v1/messages requests so llama-server doesn't reject custom model IDs.
    if !request_body.is_empty() && path_and_query.contains("/v1/messages") {
        if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&request_body) {
            if let Some(obj) = json_val.as_object_mut() {
                if obj.contains_key("model") {
                    obj.insert("model".to_string(), serde_json::Value::String("claude-sonnet-4-5".to_string()));
                    if let Ok(normalized_bytes) = serde_json::to_vec(&json_val) {
                        request_body = normalized_bytes;
                    }
                }
            }
        }
    }

    // Rewrite path for Responses API requests: /v1/responses → /v1/chat/completions.
    let effective_path = if is_responses {
        path_and_query.replacen("/v1/responses", "/v1/chat/completions", 1)
    } else {
        path_and_query.clone()
    };
    let target_url = format!("http://127.0.0.1:{}{}", target_port, effective_path);

    // Convert tiny_http method to reqwest method
    let method = match req.method().as_str() {
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        "HEAD" => reqwest::Method::HEAD,
        "OPTIONS" => reqwest::Method::OPTIONS,
        _ => reqwest::Method::GET,
    };

    // P2-1 FIX: Use the pre-built client instead of creating a new one per request.
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

    // If this is a GET /v1/models request, rewrite raw GGUF file path IDs
    // (e.g. "/mnt/orico/.../ornith-1.0-35b-Q4_K_M.gguf") into a clean model ID.
    // - Anthropic clients (no Authorization header): use "claude-sonnet-4-5"
    // - OpenAI clients (Authorization: Bearer present): use OpenAI-compatible format
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
                                        // OpenAI-compatible: preserve path as id, add object field
                                        obj.insert(
                                            "object".to_string(),
                                            serde_json::Value::String("model".to_string()),
                                        );
                                    } else {
                                        // Extract clean model ID from GGUF path
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

                                        // Primary entry: clean dynamic model ID for Claude Code CLI
                                        obj.insert(
                                            "id".to_string(),
                                            serde_json::Value::String(clean_id),
                                        );

                                        // Secondary entry: claude-sonnet-4-5 for Claude Desktop Model Discovery
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

    // Build response headers for tiny_http, stripping hop-by-hop headers
    // per RFC 7230 §6.1 to prevent response-framing edge cases.
    let mut response_headers = Vec::new();
    for (name, value) in resp_headers.iter() {
        if is_hop_by_hop_header(name.as_str()) {
            continue; // Skip hop-by-hop headers — they are connection-scoped only.
        }
        response_headers.push(
            Header::from_bytes(name.as_str(), value.as_bytes())
                .unwrap_or_else(|_| {
                    Header::from_bytes(name.as_str(), b"")
                        .expect("header construction should never fail")
                }),
        );
    }

    // Add content-length header if available
    if let Some(content_length) = response.content_length() {
        response_headers.push(
            Header::from_bytes("content-length", content_length.to_string().as_bytes())
                .unwrap_or_else(|_| {
                    Header::from_bytes("content-length", b"0")
                        .expect("header construction should never fail")
                }),
        );
    }

    // Create a streaming response that yields chunks as they arrive from the
    // backend. This preserves SSE streaming for chat completions — tokens are
    // sent to the client as soon as they're generated, rather than waiting for
    // the entire response to complete.
    let is_responses_api = path_and_query.contains("/v1/responses");

    if is_responses_api {
        // Read the full OpenAI SSE body and translate it into Responses API events.
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
            None, // data_length computed from Read trait
            None, // additional_headers
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
            None, // data_length computed from Read trait
            None, // additional_headers
        );

        if let Err(e) = req.respond(tiny_response) {
            debug!("failed to respond to proxy client: {}", e);
        }
    }
}

/// A streaming body reader that reads chunks from a backend response.
///
/// Used to preserve SSE streaming for chat completions — the proxy pipes
/// the response body through as it arrives rather than buffering the full
/// response before forwarding.
struct StreamingBody {
    reader: reqwest::blocking::Response,
    is_responses_api: bool,
    sent_completion: bool,
    leftover: Vec<u8>,
}

impl Read for StreamingBody {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if !self.leftover.is_empty() {
            let to_copy = std::cmp::min(buf.len(), self.leftover.len());
            buf[..to_copy].copy_from_slice(&self.leftover[..to_copy]);
            self.leftover.drain(..to_copy);
            return Ok(to_copy);
        }

        let n = self.reader.read(buf)?;
        if n == 0 && self.is_responses_api && !self.sent_completion {
            self.sent_completion = true;
            let completion_evt = b"\nevent: response.completed\ndata: {\"type\": \"response.completed\"}\n\n";
            let to_copy = std::cmp::min(buf.len(), completion_evt.len());
            buf[..to_copy].copy_from_slice(&completion_evt[..to_copy]);
            if completion_evt.len() > to_copy {
                self.leftover.extend_from_slice(&completion_evt[to_copy..]);
            }
            return Ok(to_copy);
        }
        Ok(n)
    }
}

/// Streaming body that translates OpenAI SSE chunks into Responses API SSE events.
///
/// Pre-parses the full OpenAI SSE response body into a sequence of Responses API
/// events (response.created, output_item.added, content_part.added, text.delta × N,
/// response.completed) and yields them one at a time via the `Read` trait.
struct ResponsesStreamingBody {
    /// Pre-formatted Responses API SSE events ready to be yielded.
    events: Vec<Vec<u8>>,
    pos: usize,
}

impl Read for ResponsesStreamingBody {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if self.pos >= self.events.len() {
            return Ok(0);
        }

        let event = &self.events[self.pos];
        self.pos += 1;

        let to_copy = std::cmp::min(buf.len(), event.len());
        buf[..to_copy].copy_from_slice(&event[..to_copy]);
        if event.len() > to_copy {
            // Split the event: keep the remainder for the next read.
            let remainder = event[to_copy..].to_vec();
            self.events.insert(self.pos, remainder);
        }
        Ok(to_copy)
    }
}

/// Translate a raw OpenAI SSE response body into Responses API SSE events.
///
/// Reads the full body, parses each `data:` line, and emits the complete
/// lifecycle: `response.created` → `output_item.added` / `content_part.added`
/// → `text.delta` × N → `response.completed`.
fn translate_openai_sse_to_responses(
    openai_sse_body: &str,
    model_id: &str,
) -> Vec<Vec<u8>> {
    let mut events: Vec<Vec<u8>> = Vec::new();

    // Generate a deterministic response ID from the model + timestamp.
    let response_id = format!("resp_{}", chrono::Utc::now().timestamp());
    let item_id = format!("msg_{}", &response_id[..std::cmp::min(response_id.len(), 16)]);
    let mut seq = 1;

    // 1. Emit `response.created` event.
    let created_event = format!(
        "event: response.created\ndata: {{\"type\":\"response.created\",\"sequence_number\":{},\"response\":{{\"id\":\"{}\",\"object\":\"response\",\"created\":{},\"model\":\"{}\",\"status\":\"in_progress\",\"output\":[]}}}}\n\n",
        seq,
        response_id,
        chrono::Utc::now().timestamp(),
        model_id,
    );
    seq += 1;
    events.push(created_event.into_bytes());

    // 2. Emit `response.output_item.added` + `response.content_part.added`.
    let item_added = format!(
        "event: response.output_item.added\ndata: {{\"type\":\"response.output_item.added\",\"sequence_number\":{},\"response_id\":\"{}\",\"output_index\":0,\"item\":{{\"id\":\"{}\",\"type\":\"message\",\"status\":\"in_progress\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"\"}}]}}}}\n\n",
        seq,
        response_id,
        item_id
    );
    seq += 1;
    events.push(item_added.into_bytes());

    let content_part_added = format!(
        "event: response.content_part.added\ndata: {{\"type\":\"response.content_part.added\",\"sequence_number\":{},\"response_id\":\"{}\",\"item_id\":\"{}\",\"output_index\":0,\"content_index\":0,\"part\":{{\"type\":\"output_text\",\"text\":\"\"}}}}\n\n",
        seq,
        response_id,
        item_id
    );
    seq += 1;
    events.push(content_part_added.into_bytes());

    // 3. Translate each OpenAI SSE chunk into `response.output_text.delta`.
    let mut accumulated_text = String::new();
    for line in openai_sse_body.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            let data = data.trim();
            if data == "[DONE]" {
                continue; // Handled by step 4.
            }
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(choices) = json_val.get("choices").and_then(|c| c.as_array()) {
                    if let Some(first) = choices.first() {
                        if let Some(delta) = first.get("delta") {
                            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                if !content.is_empty() {
                                    accumulated_text.push_str(content);
                                    let escaped = content.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r");
                                    let text_delta = format!(
                                        "event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"sequence_number\":{},\"response_id\":\"{}\",\"item_id\":\"{}\",\"output_index\":0,\"content_index\":0,\"delta\":\"{}\"}}\n\n",
                                        seq,
                                        response_id,
                                        item_id,
                                        escaped
                                    );
                                    seq += 1;
                                    events.push(text_delta.into_bytes());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let escaped_full_text = accumulated_text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r");

    // 4. Emit `response.output_text.done` + `response.content_part.done` + `response.output_item.done` + `response.completed`.
    let text_done = format!(
        "event: response.output_text.done\ndata: {{\"type\":\"response.output_text.done\",\"sequence_number\":{},\"response_id\":\"{}\",\"item_id\":\"{}\",\"output_index\":0,\"content_index\":0,\"text\":\"{}\"}}\n\n",
        seq,
        response_id,
        item_id,
        escaped_full_text
    );
    seq += 1;
    events.push(text_done.into_bytes());

    let part_done = format!(
        "event: response.content_part.done\ndata: {{\"type\":\"response.content_part.done\",\"sequence_number\":{},\"response_id\":\"{}\",\"item_id\":\"{}\",\"output_index\":0,\"content_index\":0,\"part\":{{\"type\":\"output_text\",\"text\":\"{}\"}}}}\n\n",
        seq,
        response_id,
        item_id,
        escaped_full_text
    );
    seq += 1;
    events.push(part_done.into_bytes());

    let item_done = format!(
        "event: response.output_item.done\ndata: {{\"type\":\"response.output_item.done\",\"sequence_number\":{},\"response_id\":\"{}\",\"output_index\":0,\"item\":{{\"id\":\"{}\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"{}\"}}]}}}}\n\n",
        seq,
        response_id,
        item_id,
        escaped_full_text
    );
    seq += 1;
    events.push(item_done.into_bytes());

    let completed_event = format!(
        "event: response.completed\ndata: {{\"type\":\"response.completed\",\"sequence_number\":{},\"response\":{{\"id\":\"{}\",\"object\":\"response\",\"created\":{},\"model\":\"{}\",\"status\":\"completed\",\"output\":[{{\"id\":\"{}\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"{}\"}}]}}]}}}}\n\n",
        seq,
        response_id,
        chrono::Utc::now().timestamp(),
        model_id,
        item_id,
        escaped_full_text
    );
    events.push(completed_event.into_bytes());

    events
}

/// Resolve the target port for an incoming request by inspecting its JSON body.
///
/// Checks for a `model` field in:
/// - OpenAI `/v1/chat/completions` payloads (`{"model": "..."}`)
/// - Anthropic `/v1/messages` payloads (`{"model": "..."}`)
/// - Ollama `/api/chat` and `/api/generate` payloads (`{"model": "..."}`)
///
/// If the `model` value matches a running model's id or name, returns that
/// model's port. Otherwise returns `None` so the caller falls back to the
/// primary active model.
fn resolve_target_port(state: &ProxyState, body: &[u8]) -> Option<u16> {
    // Only inspect JSON bodies.
    if body.is_empty() {
        return None;
    }

    // Quick check: does the body contain a "model" key at all?
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

    // Check against running models.
    for (id, &port) in &state.active_models {
        if id == model_id {
            return Some(port);
        }
    }

    None
}

/// Check if a header name is a hop-by-hop header per RFC 7230 §6.1.
///
/// These headers are intended for a single transport-level connection and must
/// be stripped before forwarding to prevent response-framing edge cases.
fn is_hop_by_hop_header(name: &str) -> bool {
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
fn error_response(status: u16, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
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

// ─── Ollama API translator ──────────────────────────────────────────────────

/// Ollama `/api/generate` request.
#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
struct OllamaGenerateRequest {
    model: String,
    prompt: String,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    options: Option<OllamaOptions>,
    #[serde(default)]
    system: Option<String>,
    #[serde(default)]
    images: Option<Vec<String>>,
}

/// Ollama `/api/chat` request.
#[derive(Debug, serde::Deserialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    options: Option<OllamaOptions>,
}

/// Ollama message role/content.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct OllamaMessage {
    role: String,
    content: serde_json::Value,
}

/// Ollama generation options (subset we care about).
#[derive(Debug, serde::Deserialize)]
struct OllamaOptions {
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    num_predict: Option<i64>,
    #[serde(default)]
    top_p: Option<f64>,
    #[serde(default)]
    top_k: Option<i64>,
}

/// Ollama streaming chunk for `/api/generate`.
#[derive(Debug, serde::Serialize)]
struct OllamaGenerateChunk {
    model: String,
    #[serde(rename = "created_at")]
    created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<OllamaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_duration: Option<u64>,
}

/// Ollama streaming chunk for `/api/chat`.
#[derive(Debug, serde::Serialize)]
struct OllamaChatChunk {
    model: String,
    #[serde(rename = "created_at")]
    created_at: String,
    message: OllamaMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_duration: Option<u64>,
}

/// Ollama non-streaming response for `/api/generate`.
#[derive(Debug, serde::Serialize)]
struct OllamaGenerateResponse {
    model: String,
    #[serde(rename = "created_at")]
    created_at: String,
    message: OllamaMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_duration: Option<u64>,
}

/// Ollama non-streaming response for `/api/chat`.
#[derive(Debug, serde::Serialize)]
struct OllamaChatResponse {
    model: String,
    #[serde(rename = "created_at")]
    created_at: String,
    message: OllamaMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_duration: Option<u64>,
}

/// Ollama `/api/tags` response.
#[derive(Debug, serde::Serialize)]
#[allow(dead_code)]
struct OllamaTagsResponse {
    models: Vec<OllamaTagEntry>,
}

/// Single entry in the `/api/tags` model list.
#[derive(Debug, serde::Serialize)]
struct OllamaTagEntry {
    name: String,
    #[serde(rename = "model")]
    model_id: String,
    modified_at: String,
    size: u64,
}

/// Check whether the request path is one of the Ollama endpoints we translate.
fn is_ollama_endpoint(path: &str) -> bool {
    path == "/api/generate" || path == "/api/chat" || path == "/api/tags"
}

/// Convert an Ollama generate request into an OpenAI chat-completions payload.
fn ollama_generate_to_openai(req: &OllamaGenerateRequest) -> serde_json::Value {
    let mut messages = Vec::new();

    // If a system prompt is provided, prepend it as a system message.
    if let Some(ref sys) = req.system {
        if !sys.is_empty() {
            messages.push(serde_json::json!({
                "role": "system",
                "content": sys
            }));
        }
    }

    // The Ollama prompt becomes the user message.
    messages.push(serde_json::json!({
        "role": "user",
        "content": req.prompt
    }));

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
        "stream": req.stream,
    });

    // Map Ollama options → OpenAI parameters.
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

/// Convert an Ollama chat request into an OpenAI chat-completions payload.
fn ollama_chat_to_openai(req: &OllamaChatRequest) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            // Ollama allows content to be a string or an array of content parts.
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

/// Build an Ollama `/api/tags` response from the proxy state.
fn build_ollama_tags() -> serde_json::Value {
    let models = vec![
        OllamaTagEntry {
            name: "swai-model".to_string(),
            model_id: "swai-model".to_string(),
            modified_at: chrono::Utc::now().to_rfc3339(),
            size: 0,
        },
    ];
    serde_json::json!({ "models": models })
}

/// Handle Ollama `/api/generate` — translate to OpenAI and forward.
fn handle_ollama_generate(
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

    // Read the Ollama request body.
    let mut request_body = Vec::new();
    let reader = req.as_reader();
    if let Err(e) = reader.read_to_end(&mut request_body) {
        debug!("failed to read ollama generate request body: {}", e);
        let _ = req.respond(error_response(500, "Failed to read request body"));
        return;
    }

    // Deserialize the Ollama request.
    let ollama_req: OllamaGenerateRequest = match serde_json::from_slice(&request_body) {
        Ok(r) => r,
        Err(e) => {
            debug!("failed to parse ollama generate request: {}", e);
            let _ = req.respond(error_response(400, &format!("Invalid Ollama request: {}", e)));
            return;
        }
    };

    // Convert to OpenAI format.
    let openai_body = ollama_generate_to_openai(&ollama_req);
    let openai_bytes = match serde_json::to_vec(&openai_body) {
        Ok(b) => b,
        Err(e) => {
            debug!("failed to serialize openai body: {}", e);
            let _ = req.respond(error_response(500, "Failed to serialize request"));
            return;
        }
    };

    // Forward to the model server's /v1/chat/completions endpoint.
    let target_url = format!("http://127.0.0.1:{}/v1/chat/completions", target_port);
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

    let mut request_builder = client
        .post(&target_url)
        .header("content-type", "application/json")
        .body(openai_bytes);

    for header in &forward_headers {
        let field_name = std::str::from_utf8(header.field.as_str().as_bytes()).unwrap_or("");
        // Skip content-length — we set our own.
        if field_name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        request_builder = request_builder.header(field_name, header.value.as_str());
    }

    let response = match request_builder.send() {
        Ok(resp) => resp,
        Err(e) => {
            debug!("ollama generate forward failed: {}", e);
            let _ = req.respond(error_response(503, "Model server unavailable"));
            return;
        }
    };

    let status = response.status().as_u16();
    let resp_headers = response.headers().clone();

    // Build response headers, stripping hop-by-hop.
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

    // If streaming, transform OpenAI SSE into Ollama NDJSON.
    if ollama_req.stream {
        let body_bytes = match response.bytes() {
            Ok(b) => b,
            Err(e) => {
                debug!("failed to read ollama generate response body: {}", e);
                let _ = req.respond(error_response(502, "Failed to read model response"));
                return;
            }
        };
        let body_str = String::from_utf8_lossy(&body_bytes).to_string();
        let chunks = build_ollama_generate_chunks(&ollama_req.model, &body_str);
        let streaming_body = OllamaStreamingBody { chunks, pos: 0 };

        let tiny_response = Response::new(
            tiny_http::StatusCode(status),
            response_headers,
            Box::new(streaming_body),
            None,
            None,
        );

        if let Err(e) = req.respond(tiny_response) {
            debug!("failed to respond to ollama generate client: {}", e);
        }
    } else {
        // Non-streaming: collect full response and convert.
        let content_length = response.content_length();
        match response.bytes() {
            Ok(body_bytes) => {
                if let Ok(json_val) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                    // Extract the assistant message from the OpenAI response.
                    if let Some(choices) = json_val.get("choices").and_then(|c| c.as_array()) {
                        if let Some(first) = choices.first() {
                            if let Some(msg) = first.get("message") {
                                let role = msg
                                    .get("role")
                                    .and_then(|r| r.as_str())
                                    .unwrap_or("assistant");
                                let content = msg
                                    .get("content")
                                    .and_then(|c| c.as_str())
                                    .unwrap_or("");

                                let ollama_resp = OllamaGenerateResponse {
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
                // Fallback: pass through raw bytes.
                let mut resp_headers_with_ct = response_headers.clone();
                resp_headers_with_ct.push(
                    Header::from_bytes("content-type", b"application/json")
                        .unwrap_or_else(|_| {
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
                debug!("failed to read ollama generate response: {}", e);
                let _ = req.respond(error_response(502, "Failed to read model response"));
            }
        }
    }
}

/// Handle Ollama `/api/chat` — translate to OpenAI and forward.
fn handle_ollama_chat(
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

    // Read the Ollama request body.
    let mut request_body = Vec::new();
    let reader = req.as_reader();
    if let Err(e) = reader.read_to_end(&mut request_body) {
        debug!("failed to read ollama chat request body: {}", e);
        let _ = req.respond(error_response(500, "Failed to read request body"));
        return;
    }

    // Deserialize the Ollama request.
    let ollama_req: OllamaChatRequest = match serde_json::from_slice(&request_body) {
        Ok(r) => r,
        Err(e) => {
            debug!("failed to parse ollama chat request: {}", e);
            let _ = req.respond(error_response(400, &format!("Invalid Ollama request: {}", e)));
            return;
        }
    };

    // Convert to OpenAI format.
    let openai_body = ollama_chat_to_openai(&ollama_req);
    let openai_bytes = match serde_json::to_vec(&openai_body) {
        Ok(b) => b,
        Err(e) => {
            debug!("failed to serialize openai body: {}", e);
            let _ = req.respond(error_response(500, "Failed to serialize request"));
            return;
        }
    };

    // Forward to the model server's /v1/chat/completions endpoint.
    let target_url = format!("http://127.0.0.1:{}/v1/chat/completions", target_port);
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

    // Build response headers, stripping hop-by-hop.
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

    // If streaming, transform OpenAI SSE into Ollama NDJSON.
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
        // Non-streaming: collect full response and convert.
        let content_length = response.content_length();
        match response.bytes() {
            Ok(body_bytes) => {
                if let Ok(json_val) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                    // Extract the assistant message from the OpenAI response.
                    if let Some(choices) = json_val.get("choices").and_then(|c| c.as_array()) {
                        if let Some(first) = choices.first() {
                            if let Some(msg) = first.get("message") {
                                let role = msg
                                    .get("role")
                                    .and_then(|r| r.as_str())
                                    .unwrap_or("assistant");
                                let content = msg
                                    .get("content")
                                    .and_then(|c| c.as_str())
                                    .unwrap_or("");

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
                // Fallback: pass through raw bytes.
                let mut resp_headers_with_ct = response_headers.clone();
                resp_headers_with_ct.push(
                    Header::from_bytes("content-type", b"application/json")
                        .unwrap_or_else(|_| {
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

/// Handle Ollama `/api/tags` — return a static model list.
fn handle_ollama_tags(req: Request) {
    let tags = build_ollama_tags();
    let body = serde_json::to_vec(&tags).unwrap_or_default();

    let headers = vec![
        Header::from_bytes("content-type", b"application/json").unwrap(),
        Header::from_bytes("content-length", body.len().to_string().as_bytes()).unwrap(),
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

/// Streaming body that transforms OpenAI SSE chunks into Ollama NDJSON.
struct OllamaStreamingBody {
    /// Pre-parsed Ollama chunks ready to be yielded.
    chunks: Vec<Vec<u8>>,
    pos: usize,
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
            // Split the chunk: keep the remainder for the next read.
            let remainder = chunk[to_copy..].to_vec();
            self.chunks.insert(self.pos, remainder);
        }
        Ok(to_copy)
    }
}

/// Streaming body that transforms OpenAI SSE chunks into Ollama chat NDJSON.
struct OllamaChatStreamingBody {
    /// Pre-parsed Ollama chunks ready to be yielded.
    chunks: Vec<Vec<u8>>,
    pos: usize,
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
fn build_ollama_generate_chunks(
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
    chunks
}

/// Helper to build Ollama chat streaming chunks from an OpenAI SSE response body.
fn build_ollama_chat_chunks(
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

// ─── OpenAI Responses API adapter (`POST /v1/responses`) ─────────────────────

/// Translate an incoming `POST /v1/responses` request body into a standard
/// OpenAI `chat/completions` payload for the active model server.
///
/// - Converts the Responses API `input` field (string or item array) into
///   standard OpenAI `messages`, preserving supported roles and content.
/// - Remaps incoming model identifiers to SWAI's currently active model.
/// - Strips Responses API–specific fields (`stream_options`, etc.).
pub fn responses_adapter(
    body_bytes: &[u8],
    active_model_id: &str,
) -> Result<serde_json::Value, String> {
    let mut req_obj = serde_json::from_slice::<serde_json::Value>(body_bytes)
        .map_err(|e| format!("invalid JSON in /v1/responses request: {}", e))?;

    // Extract the original model ID (may be a full OpenAI path like
    // "models/codex-latest" or just "codex-latest").
    let _original_model = req_obj
        .get("model")
        .and_then(|m| m.as_str())
        .map(String::from)
        .unwrap_or_else(|| active_model_id.to_string());

    // Remap model ID to SWAI's active model.
    req_obj["model"] = serde_json::Value::String(active_model_id.to_string());

    // Convert Responses API `input` (string or items array) → OpenAI `messages`.
    if let Some(obj) = req_obj.as_object_mut() {
        if let Some(input) = obj.get("input").cloned() {
            let messages = convert_responses_input_to_messages(&input);
            if !messages.is_empty() {
                obj.insert("messages".to_string(), serde_json::Value::Array(messages));
            }
            // Remove the `input` field — backend expects `messages`.
            obj.remove("input");
        }

        // Clean any nested array/object content inside messages so llama-server
        // receives plain string content.
        if let Some(messages) = obj.get_mut("messages").and_then(|m| m.as_array_mut()) {
            for msg in messages {
                if let Some(content) = msg.get_mut("content") {
                    if content.is_array() || content.is_object() {
                        let text = extract_text_from_item(content);
                        *content = serde_json::Value::String(text);
                    }
                }
            }
        }

        // Remove Responses API–only fields that llama-server doesn't understand.
        obj.remove("stream_options");
    }

    Ok(req_obj)
}

/// Convert a Responses API `input` value into OpenAI `messages`.
///
/// Supports:
/// - String input → single user message
/// - Array of items, each with `role` + `content` (string or typed object)
/// - Typed items like `{"type": "message", "role": "user", "content": [...]}`
fn convert_responses_input_to_messages(input: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut messages = Vec::new();

    match input {
        // Plain string → single user message.
        serde_json::Value::String(s) if !s.is_empty() => {
            messages.push(serde_json::json!({
                "role": "user",
                "content": s,
            }));
        }

        // Array of items (text, message, or other typed items).
        serde_json::Value::Array(items) => {
            for item in items {
                let role = item
                    .get("role")
                    .and_then(|r| r.as_str())
                    .unwrap_or("user");

                // Extract text content from various supported formats.
                if let Some(content) = item.get("content").or_else(|| item.get("text")) {
                    let text = match content {
                        serde_json::Value::String(s) => s.clone(),
                        _ => extract_text_from_item(content),
                    };
                    if !text.is_empty() {
                        messages.push(serde_json::json!({
                            "role": role,
                            "content": text,
                        }));
                    }
                } else if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    if !text.is_empty() {
                        messages.push(serde_json::json!({
                            "role": role,
                            "content": text,
                        }));
                    }
                } else if let Some(input_text) = item.get("input_text").and_then(|t| t.as_str()) {
                    if !input_text.is_empty() {
                        messages.push(serde_json::json!({
                            "role": role,
                            "content": input_text,
                        }));
                    }
                }
            }
        }

        _ => {}
    }

    messages
}

/// Translate an OpenAI `chat/completions` SSE stream into Responses API SSE events.
///
/// Emits the full lifecycle:
/// 1. `response.created` — at start (with response ID, model, created timestamp)
/// 2. `response.output_item.added` + `response.content_part.added` — for each message
/// 3. `response.text.delta` — for each token delta
/// 4. `response.completed` — at end of stream
///
/// Returns the translated SSE body as a byte string ready to stream to the client.
pub fn sse_responses_translator(
    openai_sse_body: &str,
    model_id: &str,
    response_id: &str,
) -> Vec<Vec<u8>> {
    let mut events: Vec<Vec<u8>> = Vec::new();

    // 1. Emit `response.created` event.
    let created_event = format!(
        "event: response.created\ndata: {{\"id\":\"{}\",\"object\":\"chat.completion.chunk\",\"created\":{},\"model\":\"{}\",\"content_prefix\":\"\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":null}}]}}\n\n",
        response_id,
        chrono::Utc::now().timestamp(),
        model_id,
    );
    events.push(created_event.into_bytes());

    // 2. Emit `response.output_item.added` + `response.content_part.added`.
    let item_added = format!(
        "event: response.output_item.added\ndata: {{\"type\":\"message\",\"id\":\"msg_{}\",\"role\":\"assistant\",\"content\":[]}}\n\n",
        &response_id[..std::cmp::min(response_id.len(), 16)]
    );
    events.push(item_added.into_bytes());

    let content_part_added = "event: response.content_part.added\ndata: {\"type\":\"text\",\"text\":\"\"}\n\n".to_string();
    events.push(content_part_added.into_bytes());

    // 3. Translate each OpenAI SSE chunk into `response.text.delta`.
    for line in openai_sse_body.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            let data = data.trim();
            if data == "[DONE]" {
                continue; // Handled by step 4.
            }
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(choices) = json_val.get("choices").and_then(|c| c.as_array()) {
                    if let Some(first) = choices.first() {
                        if let Some(delta) = first.get("delta") {
                            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                if !content.is_empty() {
                                    // Escape the content for JSON embedding.
                                    let escaped = content.replace('\\', "\\\\").replace('"', "\\\"");
                                    let text_delta = format!(
                                        "event: response.text.delta\ndata: {{\"type\":\"text\",\"delta\":\"{}\"}}\n\n",
                                        escaped
                                    );
                                    events.push(text_delta.into_bytes());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 4. Emit `response.completed` event.
    let completed_event = format!(
        "event: response.completed\ndata: {{\"id\":\"{}\",\"object\":\"chat.completion\",\"created\":{},\"model\":\"{}\",\"content\":[{{\"type\":\"text\",\"text\":\"\"}}],\"output\":[{{\"type\":\"message\",\"id\":\"msg_{}\",\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"\"}}]}}],\"usage\":{{\"prompt_tokens\":0,\"completion_tokens\":0,\"total_tokens\":0}},\"status\":\"completed\"}}\n\n",
        response_id,
        chrono::Utc::now().timestamp(),
        model_id,
        &response_id[..std::cmp::min(response_id.len(), 16)]
    );
    events.push(completed_event.into_bytes());

    events
}

/// Build a standard Responses API error JSON payload from a backend HTTP error.
///
/// Converts OpenAI-style errors (`{"error": {"message": "...", "type": "..."}}`)
/// and plain string errors into the Responses API error format:
/// `{"error": {"message": "...", "type": "invalid_request_error"}}`.
pub fn responses_error_response(status: u16, message: &str) -> Vec<u8> {
    let type_name = match status {
        400 | 401 | 403 => "invalid_request_error",
        404 => "not_found_error",
        429 => "rate_limit_error",
        500 | 502 | 503 | 504 => "server_error",
        _ => "api_error",
    };

    let body = format!(
        "{{\"error\":{{\"message\":\"{}\",\"type\":\"{}\",\"param\":null,\"code\":null}}}}",
        escape_json_string(message),
        type_name,
    );

    body.into_bytes()
}

/// Escape a string for safe embedding inside a JSON string literal.
fn escape_json_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Normalize Codex CLI / OpenAI Responses API payloads (`input`, typed item arrays)
/// into standard OpenAI `messages` format expected by `llama-server`.
///
/// Backwards-compatible wrapper around `responses_adapter()` for use in the legacy
/// non-Responses path. Preserves existing behavior for payloads that arrive via
/// `POST /v1/chat/completions` with a pre-existing `input` field.
pub fn normalize_codex_payload(val: &mut serde_json::Value) {
    if let Some(obj) = val.as_object_mut() {
        if let Some(input) = obj.get("input").cloned() {
            if obj.get("messages").is_none() {
                let messages = convert_responses_input_to_messages(&input);
                if !messages.is_empty() {
                    obj.insert("messages".to_string(), serde_json::Value::Array(messages));
                }
            }
        }

        // Clean any nested array content objects inside messages
        if let Some(messages) = obj.get_mut("messages").and_then(|m| m.as_array_mut()) {
            for msg in messages {
                if let Some(content) = msg.get_mut("content") {
                    if content.is_array() || content.is_object() {
                        let text = extract_text_from_item(content);
                        *content = serde_json::Value::String(text);
                    }
                }
            }
        }
    }
}

/// Recursively extract plain string content from typed items or content arrays.
fn extract_text_from_item(val: &serde_json::Value) -> String {
    if let Some(s) = val.as_str() {
        return s.to_string();
    }
    if let Some(arr) = val.as_array() {
        let mut parts = Vec::new();
        for sub in arr {
            let extracted = extract_text_from_item(sub);
            if !extracted.is_empty() {
                parts.push(extracted);
            }
        }
        return parts.join(" ");
    }
    if let Some(obj) = val.as_object() {
        if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
            return text.to_string();
        }
        if let Some(content) = obj.get("content") {
            return extract_text_from_item(content);
        }
        if let Some(input_text) = obj.get("input_text").and_then(|t| t.as_str()) {
            return input_text.to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_state_default() {
        let state = ProxyState::default();
        assert!(state.primary_port.is_none());
        assert!(state.active_models.is_empty());
        assert!(!state.is_loading);
    }

    #[test]
    fn test_proxy_state_set_target() {
        let mut state = ProxyState::new();
        state.set_target(8081);
        assert_eq!(state.primary_port, Some(8081));
        assert!(!state.is_loading);
    }

    #[test]
    fn test_proxy_state_set_loading() {
        let mut state = ProxyState::new();
        state.set_target(8081);
        state.set_loading();
        assert_eq!(state.primary_port, Some(8081));
        assert!(state.is_loading);
    }

    #[test]
    fn test_proxy_state_clear() {
        let mut state = ProxyState::new();
        state.set_target(8081);
        state.clear();
        assert!(state.primary_port.is_none());
        assert!(state.active_models.is_empty());
        assert!(!state.is_loading);
    }

    #[test]
    fn test_proxy_state_lifecycle() {
        let mut state = ProxyState::new();

        // Initial: no model
        assert!(state.primary_port.is_none());
        assert!(!state.is_loading);

        // Model starting
        state.set_loading();
        assert!(state.is_loading);

        // Model ready
        state.set_target(9081);
        assert_eq!(state.primary_port, Some(9081));
        assert!(!state.is_loading);

        // Model stopped
        state.clear();
        assert!(state.primary_port.is_none());
        assert!(!state.is_loading);

        // Switch to another model
        state.set_loading();
        state.set_target(9082);
        assert_eq!(state.primary_port, Some(9082));
        assert!(!state.is_loading);
    }

    #[test]
    fn test_proxy_state_multi_model_add_remove() {
        let mut state = ProxyState::new();

        // Add first model — becomes primary.
        state.add_model("model-a".to_string(), 9081);
        assert_eq!(state.primary_port, Some(9081));
        assert_eq!(state.active_models.len(), 1);

        // Add second model — primary stays the same.
        state.add_model("model-b".to_string(), 9082);
        assert_eq!(state.primary_port, Some(9081));
        assert_eq!(state.active_models.len(), 2);

        // Add third model.
        state.add_model("model-c".to_string(), 9083);
        assert_eq!(state.active_models.len(), 3);

        // Remove middle model — primary unchanged.
        state.remove_model("model-b");
        assert_eq!(state.active_models.len(), 2);
        assert_eq!(state.primary_port, Some(9081));

        // Remove primary model — primary should shift to next available.
        state.remove_model("model-a");
        assert_eq!(state.active_models.len(), 1);
        // Primary shifts to the remaining model's port.
        assert_eq!(state.primary_port, Some(9083));

        // Remove last model — everything cleared.
        state.remove_model("model-c");
        assert!(state.primary_port.is_none());
        assert!(state.active_models.is_empty());
    }

    #[test]
    fn test_proxy_state_find_model_port() {
        let mut state = ProxyState::new();
        state.add_model("llama3".to_string(), 9081);
        state.add_model("codex".to_string(), 9082);

        assert_eq!(state.find_model_port("llama3"), Some(9081));
        assert_eq!(state.find_model_port("codex"), Some(9082));
        assert_eq!(state.find_model_port("nonexistent"), None);
    }

    // ─── Dynamic multi-model routing tests (Phase 23) ──────────────────────

    #[test]
    fn test_resolve_target_port_matches_running_model() {
        let mut state = ProxyState::new();
        state.add_model("llama3.2".to_string(), 9081);
        state.add_model("codex-latest".to_string(), 9082);

        // Request targeting llama3.2 should route to 9081.
        let body = br#"{"model": "llama3.2", "messages": [{"role": "user", "content": "hi"}]}"#;
        assert_eq!(resolve_target_port(&state, body), Some(9081));

        // Request targeting codex should route to 9082.
        let body = br#"{"model": "codex-latest", "messages": [{"role": "user", "content": "hi"}]}"#;
        assert_eq!(resolve_target_port(&state, body), Some(9082));
    }

    #[test]
    fn test_resolve_target_port_falls_back_when_no_match() {
        let mut state = ProxyState::new();
        state.add_model("llama3.2".to_string(), 9081);

        // Request for unknown model falls back to None (caller uses primary).
        let body = br#"{"model": "unknown-model", "messages": [{"role": "user", "content": "hi"}]}"#;
        assert_eq!(resolve_target_port(&state, body), None);
    }

    #[test]
    fn test_resolve_target_port_empty_body() {
        let state = ProxyState::new();
        assert_eq!(resolve_target_port(&state, &[]), None);
    }

    #[test]
    fn test_resolve_target_port_no_model_field() {
        let mut state = ProxyState::new();
        state.add_model("llama3.2".to_string(), 9081);

        // Body without a model field should return None.
        let body = br#"{"messages": [{"role": "user", "content": "hi"}]}"#;
        assert_eq!(resolve_target_port(&state, body), None);
    }

    #[test]
    fn test_resolve_target_port_invalid_json() {
        let state = ProxyState::new();
        let body = b"not json at all";
        assert_eq!(resolve_target_port(&state, body), None);
    }

    #[test]
    fn test_resolve_target_port_empty_model_value() {
        let mut state = ProxyState::new();
        state.add_model("llama3.2".to_string(), 9081);

        let body = br#"{"model": "", "messages": []}"#;
        assert_eq!(resolve_target_port(&state, body), None);
    }

    #[test]
    fn test_is_hop_by_hop_header() {
        // Standard hop-by-hop headers per RFC 7230 §6.1
        assert!(is_hop_by_hop_header("Connection"));
        assert!(is_hop_by_hop_header("Keep-Alive"));
        assert!(is_hop_by_hop_header("Transfer-Encoding"));
        assert!(is_hop_by_hop_header("TE"));
        assert!(is_hop_by_hop_header("Trailer"));
        assert!(is_hop_by_hop_header("Proxy-Authenticate"));
        assert!(is_hop_by_hop_header("Proxy-Authorization"));
        assert!(is_hop_by_hop_header("Upgrade"));

        // Non-hop-by-hop headers should not be stripped
        assert!(!is_hop_by_hop_header("Content-Type"));
        assert!(!is_hop_by_hop_header("Authorization"));
        assert!(!is_hop_by_hop_header("User-Agent"));
        assert!(!is_hop_by_hop_header("Accept"));

        // Case-insensitive matching
        assert!(is_hop_by_hop_header("CONNECTION"));
        assert!(is_hop_by_hop_header("keep-alive"));
        assert!(is_hop_by_hop_header("Transfer-Encoding"));
    }

    #[test]
    fn test_error_response() {
        let response = error_response(503, "Test error");
        // Verify the response has the correct status code and content type
        assert_eq!(response.status_code(), tiny_http::StatusCode(503));
    }

    #[test]
    fn test_is_ollama_endpoint() {
        assert!(is_ollama_endpoint("/api/generate"));
        assert!(is_ollama_endpoint("/api/chat"));
        assert!(is_ollama_endpoint("/api/tags"));
        assert!(!is_ollama_endpoint("/v1/chat/completions"));
        assert!(!is_ollama_endpoint("/api/unknown"));
        assert!(!is_ollama_endpoint(""));
    }

    #[test]
    fn test_ollama_generate_to_openai() {
        let req = OllamaGenerateRequest {
            model: "llama3.2".to_string(),
            prompt: "Hello world".to_string(),
            stream: true,
            options: Some(OllamaOptions {
                temperature: Some(0.7),
                num_predict: Some(100),
                top_p: Some(0.9),
                top_k: Some(40),
            }),
            system: Some("You are a helpful assistant.".to_string()),
            images: None,
        };

        let openai = ollama_generate_to_openai(&req);
        assert_eq!(openai["model"], "llama3.2");
        assert_eq!(openai["stream"], true);
        assert_eq!(openai["temperature"], 0.7);
        assert_eq!(openai["max_tokens"], 100);
        assert_eq!(openai["top_p"], 0.9);
        assert_eq!(openai["top_k"], 40);

        // Should have system + user messages
        let messages = openai["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "You are a helpful assistant.");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "Hello world");
    }

    #[test]
    fn test_ollama_chat_to_openai() {
        let req = OllamaChatRequest {
            model: "llama3.2".to_string(),
            messages: vec![
                OllamaMessage {
                    role: "user".to_string(),
                    content: serde_json::Value::String("Hello".to_string()),
                },
                OllamaMessage {
                    role: "assistant".to_string(),
                    content: serde_json::Value::String("Hi there!".to_string()),
                },
                OllamaMessage {
                    role: "user".to_string(),
                    content: serde_json::Value::String("How are you?".to_string()),
                },
            ],
            stream: false,
            options: None,
        };

        let openai = ollama_chat_to_openai(&req);
        assert_eq!(openai["model"], "llama3.2");
        assert_eq!(openai["stream"], false);

        let messages = openai["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "Hello");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "Hi there!");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"], "How are you?");
    }

    #[test]
    fn test_build_ollama_tags() {
        let tags = build_ollama_tags();
        let models = tags["models"].as_array().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0]["name"], "swai-model");
        assert_eq!(models[0]["model"], "swai-model");
        assert!(models[0]["modified_at"].is_string());
    }

    #[test]
    fn test_build_ollama_generate_chunks() {
        let sse_response = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\ndata: [DONE]\n\n";
        let chunks = build_ollama_generate_chunks("llama3.2", sse_response);
        assert_eq!(chunks.len(), 3); // 2 content chunks + 1 done chunk

        // Each chunk is "data: {...}\n\n" — extract the JSON part.
        fn extract_json(chunk: &[u8]) -> String {
            let s = String::from_utf8_lossy(chunk);
            s.strip_prefix("data: ").unwrap().trim().to_string()
        }

        // Parse first chunk
        let first: serde_json::Value = serde_json::from_str(&extract_json(&chunks[0])).unwrap();
        assert_eq!(first["model"], "llama3.2");
        assert_eq!(first["message"]["content"], "Hello");
        assert!(first.get("done").is_none()); // done is omitted for content chunks

        // Parse second chunk
        let second: serde_json::Value = serde_json::from_str(&extract_json(&chunks[1])).unwrap();
        assert_eq!(second["message"]["content"], " world");

        // Parse done chunk
        let done: serde_json::Value = serde_json::from_str(&extract_json(&chunks[2])).unwrap();
        assert_eq!(done["done"], true);
    }

    #[test]
    fn test_build_ollama_chat_chunks() {
        let sse_response = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: [DONE]\n\n";
        let chunks = build_ollama_chat_chunks("llama3.2", sse_response);
        assert_eq!(chunks.len(), 2); // 1 content chunk + 1 done chunk

        fn extract_json(chunk: &[u8]) -> String {
            let s = String::from_utf8_lossy(chunk);
            s.strip_prefix("data: ").unwrap().trim().to_string()
        }

        let done: serde_json::Value = serde_json::from_str(&extract_json(&chunks[1])).unwrap();
        assert_eq!(done["done"], true);
        assert_eq!(done["message"]["role"], "assistant");
    }

    // ─── Responses API adapter tests ──────────────────────────────────────────

    #[test]
    fn test_responses_adapter_string_input() {
        let body = r#"{"input": "Hello world", "model": "codex-latest"}"#;
        let result = responses_adapter(body.as_bytes(), "swai-active-model").unwrap();
        assert_eq!(result["model"], "swai-active-model");
        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "Hello world");
        // input field should be removed
        assert!(result.get("input").is_none());
    }

    #[test]
    fn test_responses_adapter_items_array_input() {
        let body = r#"{
            "input": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there!"},
                {"role": "user", "content": "How are you?"}
            ],
            "model": "codex-latest"
        }"#;
        let result = responses_adapter(body.as_bytes(), "swai-active-model").unwrap();
        assert_eq!(result["model"], "swai-active-model");
        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "Hello");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "Hi there!");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"], "How are you?");
    }

    #[test]
    fn test_responses_adapter_typed_message_items() {
        let body = r#"{
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "text", "text": "Hello"}]}
            ],
            "model": "codex-latest"
        }"#;
        let result = responses_adapter(body.as_bytes(), "swai-active-model").unwrap();
        assert_eq!(result["model"], "swai-active-model");
        let messages = result["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "Hello");
    }

    #[test]
    fn test_responses_adapter_removes_input_field() {
        let body = r#"{"input": "test", "model": "x"}"#;
        let result = responses_adapter(body.as_bytes(), "m").unwrap();
        assert!(result.get("input").is_none());
        assert!(result.get("messages").is_some());
    }

    #[test]
    fn test_responses_adapter_removes_stream_options() {
        let body = r#"{"input": "test", "model": "x", "stream_options": {"include_usage": true}}"#;
        let result = responses_adapter(body.as_bytes(), "m").unwrap();
        assert!(result.get("stream_options").is_none());
    }

    #[test]
    fn test_responses_adapter_invalid_json() {
        let result = responses_adapter(b"not json", "m");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid JSON"));
    }

    #[test]
    fn test_sse_responses_translator_emits_lifecycle_events() {
        let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\ndata: [DONE]\n\n";
        let events = sse_responses_translator(sse_body, "swai-model", "resp_123");

        // Should have: created, output_item.added, content_part.added, 2x text.delta, completed
        assert!(events.len() >= 5);

        // First event should be response.created
        let first = String::from_utf8_lossy(&events[0]);
        assert!(first.starts_with("event: response.created"));

        // Should contain output_item.added
        let joined: String = events.iter().map(|e| String::from_utf8_lossy(e).to_string()).collect();
        assert!(joined.contains("event: response.output_item.added"));
        assert!(joined.contains("event: response.content_part.added"));

        // Should contain text deltas
        assert!(joined.contains("event: response.text.delta"));
        assert!(joined.contains("\"delta\":\"Hello\""));
        assert!(joined.contains("\"delta\":\" world\""));

        // Should end with response.completed
        assert!(joined.contains("event: response.completed"));
    }

    #[test]
    fn test_sse_responses_translator_escaped_content() {
        // Content with quotes that need escaping in JSON output.
        let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":\"He said \\\"hello\\\"\"}}]}\n\ndata: [DONE]\n\n";
        let events = sse_responses_translator(sse_body, "m", "resp_x");

        let joined: String = events.iter().map(|e| String::from_utf8_lossy(e).to_string()).collect();
        // The delta should contain escaped quotes (backslash-quote pairs)
        // Verify the escaping is present by checking for the backslash pattern.
        assert!(joined.contains("\\\""));
    }

    #[test]
    fn test_responses_error_response_status_mapping() {
        let err_400 = String::from_utf8(responses_error_response(400, "bad request")).unwrap();
        assert!(err_400.contains("invalid_request_error"));

        let err_401 = String::from_utf8(responses_error_response(401, "unauthorized")).unwrap();
        assert!(err_401.contains("invalid_request_error"));

        let err_404 = String::from_utf8(responses_error_response(404, "not found")).unwrap();
        assert!(err_404.contains("not_found_error"));

        let err_429 = String::from_utf8(responses_error_response(429, "rate limited")).unwrap();
        assert!(err_429.contains("rate_limit_error"));

        let err_500 = String::from_utf8(responses_error_response(500, "server error")).unwrap();
        assert!(err_500.contains("server_error"));

        let err_503 = String::from_utf8(responses_error_response(503, "unavailable")).unwrap();
        assert!(err_503.contains("server_error"));
    }

    #[test]
    fn test_responses_error_response_escapes_message() {
        let err = String::from_utf8(responses_error_response(400, "bad \"request\"\nwith newline")).unwrap();
        assert!(err.contains("\\\"request\\\""));
        assert!(err.contains("\\nwith newline"));
    }

    #[test]
    fn test_convert_responses_input_to_messages_string() {
        let input = serde_json::json!("Hello world");
        let messages = convert_responses_input_to_messages(&input);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "Hello world");
    }

    #[test]
    fn test_convert_responses_input_to_messages_empty_string() {
        let input = serde_json::json!("");
        let messages = convert_responses_input_to_messages(&input);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_convert_responses_input_to_messages_array() {
        let input = serde_json::json!([
            {"role": "system", "content": "You are helpful"},
            {"role": "user", "content": "Hi"}
        ]);
        let messages = convert_responses_input_to_messages(&input);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
    }

    #[test]
    fn test_convert_responses_input_to_messages_with_text_field() {
        let input = serde_json::json!([
            {"type": "message", "role": "user", "text": "Hello via text field"}
        ]);
        let messages = convert_responses_input_to_messages(&input);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"], "Hello via text field");
    }

    #[test]
    fn test_extract_text_from_item_string() {
        let val = serde_json::json!("plain text");
        assert_eq!(extract_text_from_item(&val), "plain text");
    }

    #[test]
    fn test_extract_text_from_item_array() {
        let val = serde_json::json!([
            {"type": "text", "text": "Hello"},
            {"type": "text", "text": "World"}
        ]);
        assert_eq!(extract_text_from_item(&val), "Hello World");
    }

    #[test]
    fn test_extract_text_from_item_nested_object() {
        let val = serde_json::json!({"content": {"text": "nested"}});
        assert_eq!(extract_text_from_item(&val), "nested");
    }

    #[test]
    fn test_translate_openai_sse_to_responses_full_lifecycle() {
        let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"B\"}}]}\n\ndata: [DONE]\n\n";
        let events = translate_openai_sse_to_responses(sse_body, "test-model");

        // Verify the full lifecycle is present.
        let joined: String = events.iter().map(|e| String::from_utf8_lossy(e).to_string()).collect();
        assert!(joined.contains("event: response.created"));
        assert!(joined.contains("event: response.output_item.added"));
        assert!(joined.contains("event: response.content_part.added"));
        assert!(joined.contains("event: response.output_text.delta"));
        assert!(joined.contains("event: response.output_text.done"));
        assert!(joined.contains("event: response.content_part.done"));
        assert!(joined.contains("event: response.output_item.done"));
        assert!(joined.contains("\"delta\":\"A\""));
        assert!(joined.contains("\"delta\":\"B\""));
        assert!(joined.contains("event: response.completed"));

        // Verify ordering: created comes before output_item.added, which comes
        // before text deltas, which come before completed.
        let created_pos = joined.find("event: response.created").unwrap();
        let item_added_pos = joined.find("event: response.output_item.added").unwrap();
        let first_delta_pos = joined.find("\"delta\":\"A\"").unwrap();
        let completed_pos = joined.find("event: response.completed").unwrap();
        assert!(created_pos < item_added_pos);
        assert!(item_added_pos < first_delta_pos);
        assert!(first_delta_pos < completed_pos);
    }

    #[test]
    fn test_normalize_codex_payload_with_input_string() {
        let mut val = serde_json::json!({"input": "Hello", "model": "x"});
        normalize_codex_payload(&mut val);
        assert_eq!(val["messages"].as_array().unwrap().len(), 1);
        assert_eq!(val["messages"][0]["content"], "Hello");
    }

    #[test]
    fn test_normalize_codex_payload_with_input_items() {
        let mut val = serde_json::json!({
            "input": [
                {"role": "user", "content": "Hi"},
                {"role": "assistant", "content": "Hello!"}
            ],
            "model": "x"
        });
        normalize_codex_payload(&mut val);
        let messages = val["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["content"], "Hi");
        assert_eq!(messages[1]["content"], "Hello!");
    }

    #[test]
    fn test_normalize_codex_payload_preserves_existing_messages() {
        let mut val = serde_json::json!({
            "messages": [{"role": "user", "content": "existing"}],
            "input": "ignored"
        });
        normalize_codex_payload(&mut val);
        assert_eq!(val["messages"].as_array().unwrap().len(), 1);
        assert_eq!(val["messages"][0]["content"], "existing");
    }

    #[test]
    fn test_normalize_codex_payload_clean_nested_content() {
        let mut val = serde_json::json!({
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "nested"}]}
            ]
        });
        normalize_codex_payload(&mut val);
        assert_eq!(val["messages"][0]["content"], "nested");
    }
}

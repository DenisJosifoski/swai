use std::io::{Read, Result as IoResult};

/// A streaming body reader that reads chunks from a backend response.
///
/// Used to preserve SSE streaming for chat completions — the proxy pipes
/// the response body through as it arrives rather than buffering the full
/// response before forwarding.
pub struct StreamingBody {
    pub reader: reqwest::blocking::Response,
    pub is_responses_api: bool,
    pub sent_completion: bool,
    pub leftover: Vec<u8>,
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
            let completion_evt =
                b"\nevent: response.completed\ndata: {\"type\": \"response.completed\"}\n\n";
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

use std::sync::mpsc::Receiver;

/// Streaming body that translates OpenAI SSE chunks into Responses API SSE events.
///
/// Pre-parses the full OpenAI SSE response body into a sequence of Responses API
/// events (response.created, output_item.added, content_part.added, text.delta × N,
/// response.completed) and yields them one at a time via the `Read` trait.
pub enum ResponsesSource {
    Events { events: Vec<Vec<u8>>, pos: usize },
    Receiver { receiver: Receiver<Vec<u8>> },
}

pub struct ResponsesStreamingBody {
    pub source: ResponsesSource,
    pub leftover: Vec<u8>,
}

impl Read for ResponsesStreamingBody {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if !self.leftover.is_empty() {
            let to_copy = std::cmp::min(buf.len(), self.leftover.len());
            buf[..to_copy].copy_from_slice(&self.leftover[..to_copy]);
            self.leftover.drain(..to_copy);
            return Ok(to_copy);
        }

        match &mut self.source {
            ResponsesSource::Events { events, pos } => {
                if *pos >= events.len() {
                    return Ok(0);
                }
                let event = &events[*pos];
                *pos += 1;
                let to_copy = std::cmp::min(buf.len(), event.len());
                buf[..to_copy].copy_from_slice(&event[..to_copy]);
                if event.len() > to_copy {
                    self.leftover.extend_from_slice(&event[to_copy..]);
                }
                Ok(to_copy)
            }
            ResponsesSource::Receiver { receiver } => match receiver.recv() {
                Ok(event) => {
                    let to_copy = std::cmp::min(buf.len(), event.len());
                    buf[..to_copy].copy_from_slice(&event[..to_copy]);
                    if event.len() > to_copy {
                        self.leftover.extend_from_slice(&event[to_copy..]);
                    }
                    Ok(to_copy)
                }
                Err(_) => Ok(0),
            },
        }
    }
}

/// Translate a raw OpenAI SSE response body into Responses API SSE events.
///
/// Reads the full body, parses each `data:` line, and emits the complete
/// lifecycle: `response.created` → `output_item.added` / `content_part.added`
/// → `text.delta` × N → `response.completed`.
pub fn translate_openai_sse_to_responses(openai_sse_body: &str, model_id: &str) -> Vec<Vec<u8>> {
    let mut events: Vec<Vec<u8>> = Vec::new();

    // Generate a deterministic response ID from the model + timestamp.
    let response_id = format!("resp_{}", chrono::Utc::now().timestamp());
    let item_id = format!(
        "msg_{}",
        &response_id[..std::cmp::min(response_id.len(), 16)]
    );
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
                                    let escaped = content
                                        .replace('\\', "\\\\")
                                        .replace('"', "\\\"")
                                        .replace('\n', "\\n")
                                        .replace('\r', "\\r");
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

    let escaped_full_text = accumulated_text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");

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
        "event: response.output_item.done\ndata: {{\"type\":\"response.output_item.done\",\"sequence_number\":{},\"response_id\":\"{}\",\"output_index\":0,\"item\":{{\"id\":\"{}\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"\"}}]}}}}\n\n",
        seq,
        response_id,
        item_id
    );
    seq += 1;
    events.push(item_done.into_bytes());

    let completed_event = format!(
        "event: response.completed\ndata: {{\"type\":\"response.completed\",\"sequence_number\":{},\"response\":{{\"id\":\"{}\",\"object\":\"response\",\"created\":{},\"model\":\"{}\",\"status\":\"completed\",\"output\":[{{\"id\":\"{}\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"\"}}]}}]}}}}\n\n",
        seq,
        response_id,
        chrono::Utc::now().timestamp(),
        model_id,
        item_id
    );
    events.push(completed_event.into_bytes());

    events
}

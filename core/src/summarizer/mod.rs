//! SWAI — LLM summarizer subsystem.

pub mod client;
pub mod formatter;
pub mod router;
#[cfg(test)]
mod tests;

pub use client::{build_summarizer_request, call_summarizer, parse_summarizer_response};
pub use formatter::{format_messages_for_summarization, truncate_text};
pub use router::{resolve_summarizer_route, summarize_dropped_slice, SummarizerRoute};

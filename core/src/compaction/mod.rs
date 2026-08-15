//! SWAI — Context compaction and session checkpointing module.

pub mod eviction;
pub mod extractor;
pub mod types;
#[cfg(test)]
mod tests_anthropic;
#[cfg(test)]
mod tests_basic;

pub use eviction::{compact_messages_anthropic, inject_checkpoint_into_payload};
pub use extractor::{build_eviction_units, extract_action_lines, serialize_dropped_slice};
pub use types::{CompactionConfig, Message};

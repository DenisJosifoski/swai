//! SWAI — Context compaction and session checkpointing module.

pub mod budget;
pub mod eviction;
pub mod extractor;
#[cfg(test)]
mod tests_anthropic;
#[cfg(test)]
mod tests_basic;
pub mod types;

pub use budget::ContextBudget;
pub use budget::{DEFAULT_THRESHOLD_PCT, MAX_THRESHOLD_PCT, MIN_THRESHOLD_PCT};
pub use eviction::{
    compact_messages_anthropic, compact_messages_with_budget, inject_checkpoint_into_payload,
};
pub use extractor::{build_eviction_units, extract_action_lines, serialize_dropped_slice};
pub use types::{CompactionConfig, Message};

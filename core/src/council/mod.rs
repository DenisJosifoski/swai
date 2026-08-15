//! SWAI — Council multi-agent orchestration module.
//!
//! Defines data types, pipeline configuration, and debate transcript
//! structures for coordinating multiple LLM agents in a council pattern.

pub mod types;
pub mod vram;

#[cfg(test)]
mod tests;

// Reserved modules for future phases.
// pub mod pipeline;
// pub mod streaming;

pub use types::{
    CouncilMode, CouncilPipelineConfig, CouncilRole, DebateTranscript, FallbackAction,
    PipelineStage, TurnResult,
};
pub use vram::{get_available_vram_bytes, recommend_mode};

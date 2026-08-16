//! SWAI — Session checkpointing and persistence subsystem.

pub mod entry;
pub mod registry;
#[cfg(test)]
mod tests;
pub mod writer;

pub use entry::{CheckpointEntry, SessionCheckpoint};
pub use registry::CheckpointRegistry;
pub use writer::CheckpointWriter;

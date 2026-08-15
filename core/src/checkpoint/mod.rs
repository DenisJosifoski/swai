//! SWAI — Session checkpointing and persistence subsystem.

pub mod entry;
pub mod registry;
pub mod writer;
#[cfg(test)]
mod tests;

pub use entry::{CheckpointEntry, SessionCheckpoint};
pub use registry::CheckpointRegistry;
pub use writer::CheckpointWriter;

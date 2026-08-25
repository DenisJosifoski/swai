//! SWAI — Import wizard helpers for parsing and syncing model launch scripts.
//!
//! Provides context-size detection and bidirectional sync between
//! `config.toml` and `.sh` launch scripts.

pub mod inference;

pub use inference::{detect_ctx_size, sync_ctx_size_in_script};

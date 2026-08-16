//! SWAI — Arena module: GTK4 debate arena desktop window.
//!
//! Provides a visual interface for viewing council debate transcripts,
//! browsing saved debates, and persisting transcript data to disk.

pub mod history;
pub mod view;
pub mod window;

pub use window::ArenaWindow;

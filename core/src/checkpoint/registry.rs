use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use super::entry::SessionCheckpoint;

pub struct CheckpointRegistry {
    pub sessions: Arc<Mutex<HashMap<String, SessionCheckpoint>>>,
}

impl CheckpointRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Look up or create a session checkpoint for the given session ID.
    pub fn get_or_create(&self, session_id: &str) -> SessionCheckpoint {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionCheckpoint::new(session_id.to_string()))
            .clone()
    }

    /// Look up a session checkpoint by ID, returning a cloned copy if found.
    pub fn get(&self, session_id: &str) -> Option<SessionCheckpoint> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.get(session_id).cloned()
    }

    /// Remove a session checkpoint by ID.
    pub fn remove(&self, session_id: &str) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.remove(session_id);
    }

    /// Clear all session checkpoints from the registry.
    pub fn clear(&self) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.clear();
    }

    /// Format all active session checkpoints for injection.
    pub fn format_all(&self) -> Vec<(String, String)> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .iter()
            .filter_map(|(id, sc)| {
                sc.format_for_injection().map(|text| (id.clone(), text))
            })
            .collect()
    }
}

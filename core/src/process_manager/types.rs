use super::guard::ProcessGuard;

/// The current state of a model.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelState {
    Stopped,
    Starting,
    Loading,
    Ready,
    Error(String),
}

/// A running model's metadata.
pub struct RunningModel {
    pub id: String,
    pub guard: Box<dyn ProcessGuard>,
    pub state: ModelState,
}

impl std::fmt::Debug for RunningModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningModel")
            .field("id", &self.id)
            .field("state", &self.state)
            .finish()
    }
}

/// Port state for health checking and conflict resolution.
#[derive(Debug, Clone, PartialEq)]
pub enum PortState {
    Free,
    OccupiedByModel,
    OccupiedByUnknown(u32),
}

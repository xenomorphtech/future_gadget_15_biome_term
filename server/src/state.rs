use crate::pane::Pane;
use dashmap::DashMap;
use std::sync::Arc;
use uuid::Uuid;

pub type PaneMap = DashMap<Uuid, Arc<Pane>>;

#[derive(Clone)]
pub struct AppState {
    pub panes: Arc<PaneMap>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            panes: Arc::new(PaneMap::new()),
        }
    }
}

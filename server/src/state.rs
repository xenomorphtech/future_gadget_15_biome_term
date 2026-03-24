use crate::{pane::Pane, pane_lifecycle::PaneLifecycleEvent};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use uuid::Uuid;

pub type PaneMap = DashMap<Uuid, Arc<Pane>>;

#[derive(Clone)]
pub struct AppState {
    pub panes: Arc<PaneMap>,
    pub pane_lifecycle_tx: broadcast::Sender<PaneLifecycleEvent>,
}

impl AppState {
    pub fn new() -> Self {
        let (pane_lifecycle_tx, _) = broadcast::channel(1024);

        AppState {
            panes: Arc::new(PaneMap::new()),
            pane_lifecycle_tx,
        }
    }
}

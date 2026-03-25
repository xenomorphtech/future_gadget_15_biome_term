use crate::{pane::Pane, pane_lifecycle::PaneLifecycleEvent};
use dashmap::DashMap;
use std::{env, sync::Arc};
use tokio::sync::broadcast;
use uuid::Uuid;

pub type PaneMap = DashMap<Uuid, Arc<Pane>>;

#[derive(Clone)]
pub struct AppState {
    pub panes: Arc<PaneMap>,
    pub pane_lifecycle_tx: broadcast::Sender<PaneLifecycleEvent>,
    api_key: Option<Arc<str>>,
}

impl AppState {
    pub fn new() -> Self {
        Self::with_api_key(None)
    }

    pub fn from_env() -> Self {
        Self::with_api_key(
            env::var("BIOME_API_KEY")
                .ok()
                .and_then(|value| normalize_optional_string(Some(value))),
        )
    }

    pub fn with_api_key(api_key: Option<String>) -> Self {
        let (pane_lifecycle_tx, _) = broadcast::channel(1024);

        AppState {
            panes: Arc::new(PaneMap::new()),
            pane_lifecycle_tx,
            api_key: api_key.map(Arc::<str>::from),
        }
    }

    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

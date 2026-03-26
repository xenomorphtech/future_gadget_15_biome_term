use crate::{error::AppError, pane::Pane, pane_lifecycle::PaneLifecycleEvent};
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

    pub fn resolve_pane_id(&self, id_or_name: &str) -> Result<Uuid, AppError> {
        if let Ok(id) = Uuid::parse_str(id_or_name) {
            if self.panes.contains_key(&id) {
                return Ok(id);
            }
        }

        let mut matching_ids = self.panes.iter().filter_map(|entry| {
            (entry.value().name.as_deref() == Some(id_or_name)).then_some(*entry.key())
        });

        match (matching_ids.next(), matching_ids.next()) {
            (Some(id), None) => Ok(id),
            (Some(_), Some(_)) => Err(AppError::BadRequest(format!(
                "multiple panes named {id_or_name} found"
            ))),
            _ => Err(AppError::NotFound(format!("pane {id_or_name} not found"))),
        }
    }

    pub fn get_pane(&self, id_or_name: &str) -> Result<Arc<Pane>, AppError> {
        let id = self.resolve_pane_id(id_or_name)?;

        self.panes
            .get(&id)
            .map(|pane| pane.clone())
            .ok_or_else(|| AppError::NotFound(format!("pane {id_or_name} not found")))
    }

    pub fn remove_pane(&self, id_or_name: &str) -> Result<(Uuid, Arc<Pane>), AppError> {
        let id = self.resolve_pane_id(id_or_name)?;

        self.panes
            .remove(&id)
            .ok_or_else(|| AppError::NotFound(format!("pane {id_or_name} not found")))
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

use crate::{
    error::AppError,
    event::DEFAULT_MAX_EVENTS,
    pane::{Pane, PaneSize},
    pane_lifecycle::PaneLifecycleEvent,
};
use dashmap::DashMap;
use std::{
    env,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock,
    },
};
use tokio::sync::broadcast;
use uuid::Uuid;

pub type PaneMap = DashMap<Uuid, Arc<Pane>>;

#[derive(Clone)]
pub struct AppState {
    pub panes: Arc<PaneMap>,
    pub pane_lifecycle_tx: broadcast::Sender<PaneLifecycleEvent>,
    api_key: Option<Arc<str>>,
    pub default_max_events: Arc<AtomicUsize>,
    pub default_pane_size: Arc<RwLock<PaneSize>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
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
        let max_events = default_max_events_from_env();

        AppState {
            panes: Arc::new(PaneMap::new()),
            pane_lifecycle_tx,
            api_key: api_key.map(Arc::<str>::from),
            default_max_events: Arc::new(AtomicUsize::new(max_events)),
            default_pane_size: Arc::new(RwLock::new(PaneSize {
                cols: 220,
                rows: 50,
            })),
        }
    }

    pub fn get_default_max_events(&self) -> usize {
        self.default_max_events.load(Ordering::Relaxed)
    }

    pub fn set_default_max_events(&self, n: usize) {
        self.default_max_events.store(n, Ordering::Relaxed);
    }

    pub fn get_default_pane_size(&self) -> PaneSize {
        *self
            .default_pane_size
            .read()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_default_pane_size(&self, size: PaneSize) {
        *self
            .default_pane_size
            .write()
            .unwrap_or_else(|e| e.into_inner()) = size;
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

fn default_max_events_from_env() -> usize {
    match env::var("BIOME_DEFAULT_MAX_EVENTS") {
        Ok(value) => match parse_default_max_events(&value) {
            Ok(parsed) => parsed,
            Err(error) => {
                eprintln!(
                    "Ignoring invalid BIOME_DEFAULT_MAX_EVENTS={value:?}: {error}; using default {DEFAULT_MAX_EVENTS}"
                );
                DEFAULT_MAX_EVENTS
            }
        },
        Err(env::VarError::NotPresent) => DEFAULT_MAX_EVENTS,
        Err(error) => {
            eprintln!(
                "Failed to read BIOME_DEFAULT_MAX_EVENTS: {error}; using default {DEFAULT_MAX_EVENTS}"
            );
            DEFAULT_MAX_EVENTS
        }
    }
}

fn parse_default_max_events(value: &str) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(0) => Err("value must be >= 1".to_string()),
        Ok(parsed) => Ok(parsed),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_default_max_events;

    #[test]
    fn parse_default_max_events_rejects_zero() {
        assert_eq!(
            parse_default_max_events("0"),
            Err("value must be >= 1".to_string())
        );
    }

    #[test]
    fn parse_default_max_events_rejects_invalid_input() {
        assert!(parse_default_max_events("abc").is_err());
    }

    #[test]
    fn parse_default_max_events_accepts_positive_values() {
        assert_eq!(parse_default_max_events("2048"), Ok(2048));
    }
}

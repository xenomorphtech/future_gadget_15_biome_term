use crate::{pane::Pane, state::AppState};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use utoipa::ToSchema;
use uuid::Uuid;

/// Summary of an active pane.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct PaneInfo {
    /// Unique pane identifier
    pub id: Uuid,
    /// Human-readable label, if provided at creation
    pub name: Option<String>,
    pub cols: u16,
    pub rows: u16,
    /// True when the shell process has exited
    pub terminated: bool,
}

impl PaneInfo {
    pub fn from_pane(pane: &Pane) -> Self {
        PaneInfo {
            id: pane.id,
            name: pane.name.clone(),
            cols: pane.cols,
            rows: pane.rows,
            terminated: pane.terminated.load(Ordering::Relaxed),
        }
    }
}

pub fn list_pane_infos(state: &AppState) -> Vec<PaneInfo> {
    state
        .panes
        .iter()
        .map(|entry| PaneInfo::from_pane(entry.value()))
        .collect()
}

/// List all active panes.
#[utoipa::path(
    get,
    path = "/panes",
    responses(
        (status = 200, description = "Array of active panes", body = Vec<PaneInfo>),
    )
)]
pub async fn list_panes_handler(State(state): State<AppState>) -> Json<Vec<PaneInfo>> {
    let panes = list_pane_infos(&state);

    Json(panes)
}

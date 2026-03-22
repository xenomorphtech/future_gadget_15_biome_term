use crate::state::AppState;
use axum::{extract::State, Json};
use serde::Serialize;
use std::sync::atomic::Ordering;
use utoipa::ToSchema;
use uuid::Uuid;

/// Summary of an active pane.
#[derive(Serialize, ToSchema)]
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

/// List all active panes.
#[utoipa::path(
    get,
    path = "/panes",
    responses(
        (status = 200, description = "Array of active panes", body = Vec<PaneInfo>),
    )
)]
pub async fn list_panes_handler(
    State(state): State<AppState>,
) -> Json<Vec<PaneInfo>> {
    let panes: Vec<PaneInfo> = state
        .panes
        .iter()
        .map(|entry| PaneInfo {
            id: entry.id,
            name: entry.name.clone(),
            cols: entry.cols,
            rows: entry.rows,
            terminated: entry.terminated.load(Ordering::Relaxed),
        })
        .collect();
    Json(panes)
}

use crate::state::AppState;
use axum::{extract::State, Json};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
pub struct PaneInfo {
    pub id: Uuid,
    pub cols: u16,
    pub rows: u16,
}

pub async fn list_panes_handler(
    State(state): State<AppState>,
) -> Json<Vec<PaneInfo>> {
    let panes: Vec<PaneInfo> = state
        .panes
        .iter()
        .map(|entry| PaneInfo {
            id: entry.id,
            cols: entry.cols,
            rows: entry.rows,
        })
        .collect();
    Json(panes)
}

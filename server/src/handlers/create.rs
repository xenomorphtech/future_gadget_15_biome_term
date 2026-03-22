use crate::{error::AppError, pane::create_pane, state::AppState};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CreatePaneRequest {
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub shell: Option<String>,
}

#[derive(Serialize)]
pub struct CreatePaneResponse {
    pub id: Uuid,
    pub cols: u16,
    pub rows: u16,
}

pub async fn create_pane_handler(
    State(state): State<AppState>,
    Json(body): Json<CreatePaneRequest>,
) -> Result<Json<CreatePaneResponse>, AppError> {
    let cols = body.cols.unwrap_or(220);
    let rows = body.rows.unwrap_or(50);
    let shell = body.shell;

    let pane = create_pane(cols, rows, shell)
        .map_err(|e| AppError::Internal(e))?;

    let id = pane.id;
    state.panes.insert(id, pane);

    Ok(Json(CreatePaneResponse { id, cols, rows }))
}

use crate::{error::AppError, pane::create_pane, state::AppState};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Request body for creating a new pane.
#[derive(Deserialize, ToSchema)]
pub struct CreatePaneRequest {
    /// Human-readable label for this pane (optional)
    pub name: Option<String>,
    /// Terminal width in columns (default: 220)
    pub cols: Option<u16>,
    /// Terminal height in rows (default: 50)
    pub rows: Option<u16>,
    /// Shell executable path (default: /bin/bash)
    pub shell: Option<String>,
}

/// Created pane descriptor.
#[derive(Serialize, ToSchema)]
pub struct CreatePaneResponse {
    /// Unique pane identifier
    pub id: Uuid,
    /// Human-readable label, if provided at creation
    pub name: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

/// Create a new PTY pane.
///
/// Spawns a shell process attached to a pseudo-terminal. The pane ID is used
/// in all subsequent requests. The VT100 emulator begins processing output
/// immediately; connect via `/panes/{id}/stream` to receive live updates.
#[utoipa::path(
    post,
    path = "/panes",
    request_body = CreatePaneRequest,
    responses(
        (status = 200, description = "Pane created", body = CreatePaneResponse),
        (status = 500, description = "Failed to open PTY or spawn shell"),
    )
)]
pub async fn create_pane_handler(
    State(state): State<AppState>,
    Json(body): Json<CreatePaneRequest>,
) -> Result<Json<CreatePaneResponse>, AppError> {
    let cols = body.cols.unwrap_or(220);
    let rows = body.rows.unwrap_or(50);

    let pane = create_pane(cols, rows, body.shell, body.name)
        .map_err(|e| AppError::Internal(e))?;

    let id = pane.id;
    let name = pane.name.clone();
    state.panes.insert(id, pane);

    Ok(Json(CreatePaneResponse { id, name, cols, rows }))
}

use crate::{
    error::AppError,
    handlers::list::PaneInfo,
    pane::{create_pane, PaneSize},
    pane_lifecycle::PaneLifecycleEvent,
    state::AppState,
};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Request body for creating a new pane.
#[derive(Deserialize, ToSchema)]
pub struct CreatePaneRequest {
    /// Human-readable label for this pane (optional)
    pub name: Option<String>,
    /// Process group tag (e.g. domain name)
    pub group: Option<String>,
    /// Terminal width in columns (default: server-configured default, initially 220)
    pub cols: Option<u16>,
    /// Terminal height in rows (default: server-configured default, initially 50)
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
    /// Process group tag
    pub group: Option<String>,
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
        (status = 400, description = "Invalid pane dimensions"),
        (status = 500, description = "Failed to open PTY or spawn shell"),
    )
)]
pub async fn create_pane_handler(
    State(state): State<AppState>,
    Json(body): Json<CreatePaneRequest>,
) -> Result<Json<CreatePaneResponse>, AppError> {
    let default_size = state.get_default_pane_size();
    let size = PaneSize {
        cols: body.cols.unwrap_or(default_size.cols),
        rows: body.rows.unwrap_or(default_size.rows),
    }
    .validate()
    .map_err(AppError::BadRequest)?;

    let max_events = state.get_default_max_events();
    let pane = create_pane(size, body.shell, body.name, body.group, max_events)
        .map_err(AppError::Internal)?;

    let id = pane.id;
    let name = pane.name.clone();
    let group = pane.group();
    let pane_info = PaneInfo::from_pane(&pane);
    state.panes.insert(id, pane);
    let _ = state
        .pane_lifecycle_tx
        .send(PaneLifecycleEvent::Created { pane: pane_info });

    Ok(Json(CreatePaneResponse {
        id,
        name,
        group,
        cols: size.cols,
        rows: size.rows,
    }))
}

use crate::{
    error::AppError,
    pane::{resize_pane, PaneSize},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

/// New terminal dimensions.
#[derive(Deserialize, ToSchema)]
pub struct ResizeRequest {
    pub cols: u16,
    pub rows: u16,
}

/// Resize a pane's terminal dimensions.
///
/// Resizes the PTY master (triggering SIGWINCH so the shell redraws), then
/// replaces the VT100 parser with a fresh instance at the new size.
/// The shell prompt is typically redrawn within milliseconds.
#[utoipa::path(
    post,
    path = "/panes/{id}/resize",
    params(
        ("id" = Uuid, Path, description = "Pane ID"),
    ),
    request_body = ResizeRequest,
    responses(
        (status = 204, description = "Pane resized"),
        (status = 400, description = "Invalid pane dimensions"),
        (status = 404, description = "Pane not found"),
        (status = 500, description = "PTY resize syscall failed"),
    )
)]
pub async fn resize_pane_handler(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(body): Json<ResizeRequest>,
) -> Result<StatusCode, AppError> {
    let pane = state
        .panes
        .get(&id)
        .map(|r| r.clone())
        .ok_or_else(|| AppError::NotFound(format!("pane {id} not found")))?;

    let size = PaneSize {
        cols: body.cols,
        rows: body.rows,
    }
    .validate()
    .map_err(AppError::BadRequest)?;

    resize_pane(&pane, size).await.map_err(AppError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

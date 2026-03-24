use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use portable_pty::PtySize;
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

    // Resize the PTY master — triggers SIGWINCH so shell redraws
    {
        let master = pane.master.lock().await;
        master
            .resize(PtySize {
                rows: body.rows,
                cols: body.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| AppError::Internal(format!("resize failed: {e}")))?;
    }

    // Resize the vt100 parser's screen in place — preserves terminal state
    {
        let mut parser = pane.parser.write().await;
        parser.screen_mut().set_size(body.rows, body.cols);
    }

    Ok(StatusCode::NO_CONTENT)
}

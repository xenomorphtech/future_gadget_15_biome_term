use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use portable_pty::PtySize;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct ResizeRequest {
    pub cols: u16,
    pub rows: u16,
}

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

    // Replace the vt100 parser — it has no resize() method
    {
        let mut parser = pane.parser.write().await;
        *parser = vt100::Parser::new(body.rows, body.cols, 0);
    }

    Ok(StatusCode::NO_CONTENT)
}

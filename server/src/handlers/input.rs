use crate::{error::AppError, event::now_ms, state::AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use std::io::Write;
use std::sync::atomic::Ordering;
use utoipa::ToSchema;

/// Input to write to a pane's PTY stdin.
#[derive(Deserialize, ToSchema)]
pub struct InputRequest {
    /// Base64-encoded bytes to write verbatim to the PTY (supports escape sequences)
    pub data: String,
}

/// Write bytes to a pane's PTY stdin.
///
/// `data` must be base64-encoded. Any byte sequence is accepted, including
/// ANSI/VT escape sequences (e.g. `\x1b[A` for arrow-up).
#[utoipa::path(
    post,
    path = "/panes/{id}/input",
    params(
        ("id" = String, Path, description = "Pane ID or unique pane name"),
    ),
    request_body = InputRequest,
    responses(
        (status = 204, description = "Input written to PTY"),
        (status = 400, description = "Invalid base64 or pane name is ambiguous"),
        (status = 404, description = "Pane not found"),
    )
)]
pub async fn send_input_handler(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(body): Json<InputRequest>,
) -> Result<StatusCode, AppError> {
    let pane = state.get_pane(&id)?;

    let bytes = STANDARD
        .decode(&body.data)
        .map_err(|e| AppError::BadRequest(format!("invalid base64: {e}")))?;

    let mut writer = pane.writer.lock().await;
    writer
        .write_all(&bytes)
        .map_err(|e| AppError::Internal(format!("write failed: {e}")))?;
    pane.last_activity_ms.store(now_ms(), Ordering::Relaxed);

    Ok(StatusCode::NO_CONTENT)
}

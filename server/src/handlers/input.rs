use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use std::io::Write;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct InputRequest {
    pub data: String, // base64-encoded
}

pub async fn send_input_handler(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(body): Json<InputRequest>,
) -> Result<StatusCode, AppError> {
    let pane = state
        .panes
        .get(&id)
        .map(|r| r.clone())
        .ok_or_else(|| AppError::NotFound(format!("pane {id} not found")))?;

    let bytes = STANDARD
        .decode(&body.data)
        .map_err(|e| AppError::BadRequest(format!("invalid base64: {e}")))?;

    let mut writer = pane.writer.lock().await;
    writer
        .write_all(&bytes)
        .map_err(|e| AppError::Internal(format!("write failed: {e}")))?;

    Ok(StatusCode::NO_CONTENT)
}

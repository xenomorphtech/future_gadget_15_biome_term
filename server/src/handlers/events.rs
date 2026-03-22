use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct EventsQuery {
    pub after: Option<u64>,
}

#[derive(Serialize)]
pub struct EventResponse {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub data: String, // base64-encoded
}

pub async fn get_events_handler(
    Path(id): Path<Uuid>,
    Query(query): Query<EventsQuery>,
    State(state): State<AppState>,
) -> Result<Json<Vec<EventResponse>>, AppError> {
    let pane = state
        .panes
        .get(&id)
        .map(|r| r.clone())
        .ok_or_else(|| AppError::NotFound(format!("pane {id} not found")))?;

    let after_seq = query.after.unwrap_or(0);
    let log = pane.event_log.read().await;
    let events: Vec<EventResponse> = log
        .since(after_seq)
        .into_iter()
        .map(|e| EventResponse {
            seq: e.seq,
            timestamp_ms: e.timestamp_ms,
            data: STANDARD.encode(&e.data),
        })
        .collect();

    Ok(Json(events))
}

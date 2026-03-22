use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Query parameters for the events endpoint.
#[derive(Deserialize, IntoParams)]
pub struct EventsQuery {
    /// Return only events with `seq` greater than this value. Use `0` (default) to get all events.
    pub after: Option<u64>,
}

/// A single PTY output event.
#[derive(Serialize, ToSchema)]
pub struct EventResponse {
    /// Monotonically increasing sequence number (1-indexed)
    pub seq: u64,
    /// Unix timestamp in milliseconds
    pub timestamp_ms: u64,
    /// Base64-encoded raw PTY output bytes
    pub data: String,
}

/// Fetch PTY output events for a pane.
///
/// Returns the append-only event log since sequence number `after`.
/// Sequence numbers are 1-indexed; `after=0` returns all events.
/// For a live stream use `GET /panes/{id}/stream` (WebSocket).
#[utoipa::path(
    get,
    path = "/panes/{id}/events",
    params(
        ("id" = Uuid, Path, description = "Pane ID"),
        EventsQuery,
    ),
    responses(
        (status = 200, description = "Events since `after`", body = Vec<EventResponse>),
        (status = 404, description = "Pane not found"),
    )
)]
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

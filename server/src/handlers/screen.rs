use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, State},
    Json,
};
use serde::Serialize;
use utoipa::ToSchema;

/// Authoritative VT100 screen state of a pane.
#[derive(Serialize, ToSchema)]
pub struct ScreenResponse {
    /// One string per terminal row, trailing whitespace trimmed
    pub rows: Vec<String>,
    /// Zero-based cursor row
    pub cursor_row: u16,
    /// Zero-based cursor column
    pub cursor_col: u16,
    pub num_rows: u16,
    pub num_cols: u16,
}

/// Get the current screen state of a pane.
///
/// Returns the authoritative VT100-emulated screen buffer. Each entry in
/// `rows` is one terminal line with trailing whitespace stripped.
/// This is a snapshot; subscribe to `/panes/{id}/stream` for live updates.
#[utoipa::path(
    get,
    path = "/panes/{id}/screen",
    params(
        ("id" = String, Path, description = "Pane ID or unique pane name"),
    ),
    responses(
        (status = 200, description = "Current screen state", body = ScreenResponse),
        (status = 400, description = "Pane name is ambiguous"),
        (status = 404, description = "Pane not found"),
    )
)]
pub async fn get_screen_handler(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<ScreenResponse>, AppError> {
    let pane = state.get_pane(&id)?;

    let parser = pane.parser.read().await;
    let screen = parser.screen();
    let (num_rows, num_cols) = screen.size();
    let (cursor_row, cursor_col) = screen.cursor_position();

    // rows(start_col, width) — first arg is starting column
    let rows: Vec<String> = screen
        .rows(0, num_cols)
        .map(|r| r.trim_end().to_string())
        .collect();

    Ok(Json(ScreenResponse {
        rows,
        cursor_row,
        cursor_col,
        num_rows,
        num_cols,
    }))
}

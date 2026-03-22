use crate::{error::AppError, state::AppState};
use axum::{extract::{Path, State}, Json};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
pub struct ScreenResponse {
    pub rows: Vec<String>,
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub num_rows: u16,
    pub num_cols: u16,
}

pub async fn get_screen_handler(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<ScreenResponse>, AppError> {
    let pane = state
        .panes
        .get(&id)
        .map(|r| r.clone())
        .ok_or_else(|| AppError::NotFound(format!("pane {id} not found")))?;

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

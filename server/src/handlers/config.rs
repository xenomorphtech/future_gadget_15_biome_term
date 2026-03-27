use crate::{
    error::AppError,
    pane::{resize_pane, PaneSize},
    state::AppState,
};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Server configuration.
#[derive(Serialize, ToSchema)]
pub struct ConfigResponse {
    /// Default maximum number of events retained per pane.
    pub default_max_events: usize,
    /// Default terminal width in columns for newly created panes.
    pub default_cols: u16,
    /// Default terminal height in rows for newly created panes.
    pub default_rows: u16,
}

/// Partial update to server configuration.
#[derive(Deserialize, ToSchema)]
pub struct ConfigUpdateRequest {
    /// New default maximum number of events retained per pane (min: 100).
    pub default_max_events: Option<usize>,
    /// New default terminal width in columns for newly created panes.
    pub default_cols: Option<u16>,
    /// New default terminal height in rows for newly created panes.
    pub default_rows: Option<u16>,
    /// When true, also resize all existing panes to the resulting default size.
    #[serde(default)]
    pub apply_to_existing: bool,
}

/// Get the current server configuration.
///
/// Returns the defaults used when `POST /panes` omits `cols` and `rows`, plus
/// the default per-pane event log retention limit.
#[utoipa::path(
    get,
    path = "/config",
    responses(
        (status = 200, description = "Current configuration", body = ConfigResponse),
    )
)]
pub async fn get_config_handler(State(state): State<AppState>) -> Json<ConfigResponse> {
    let size = state.get_default_pane_size();
    Json(ConfigResponse {
        default_max_events: state.get_default_max_events(),
        default_cols: size.cols,
        default_rows: size.rows,
    })
}

/// Update server configuration.
///
/// Changes take effect for newly created panes. When `apply_to_existing` is set,
/// the resulting default terminal size is also applied to running panes
/// immediately, using the same resize path as `POST /panes/{id}/resize`.
/// Existing panes keep their current event log limits; `default_max_events`
/// only affects panes created after the update.
#[utoipa::path(
    patch,
    path = "/config",
    request_body = ConfigUpdateRequest,
    responses(
        (status = 200, description = "Updated configuration", body = ConfigResponse),
        (status = 400, description = "Invalid configuration values"),
        (status = 500, description = "Failed to resize an existing pane"),
    )
)]
pub async fn update_config_handler(
    State(state): State<AppState>,
    Json(body): Json<ConfigUpdateRequest>,
) -> Result<Json<ConfigResponse>, AppError> {
    if let Some(n) = body.default_max_events {
        let n = n.max(100);
        state.set_default_max_events(n);
    }

    let current_size = state.get_default_pane_size();
    let new_size = PaneSize {
        cols: body.default_cols.unwrap_or(current_size.cols),
        rows: body.default_rows.unwrap_or(current_size.rows),
    }
    .validate()
    .map_err(AppError::BadRequest)?;

    state.set_default_pane_size(new_size);

    if body.apply_to_existing {
        let panes: Vec<_> = state
            .panes
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        for pane in panes {
            resize_pane(&pane, new_size)
                .await
                .map_err(AppError::Internal)?;
        }
    }

    Ok(Json(ConfigResponse {
        default_max_events: state.get_default_max_events(),
        default_cols: new_size.cols,
        default_rows: new_size.rows,
    }))
}

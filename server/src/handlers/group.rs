use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use utoipa::ToSchema;

/// Request body for updating a pane's group.
#[derive(Deserialize, ToSchema)]
pub struct GroupUpdateRequest {
    /// New group name, or null to clear the group
    pub group: Option<String>,
}

/// Update the group of a pane.
#[utoipa::path(
    put,
    path = "/panes/{id}/group",
    request_body = GroupUpdateRequest,
    params(
        ("id" = String, Path, description = "Pane ID or unique pane name"),
    ),
    responses(
        (status = 204, description = "Group updated"),
        (status = 400, description = "Pane name is ambiguous"),
        (status = 404, description = "Pane not found"),
    )
)]
pub async fn update_group_handler(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(body): Json<GroupUpdateRequest>,
) -> Result<StatusCode, AppError> {
    let pane = state.get_pane(&id)?;
    pane.set_group(body.group);
    Ok(StatusCode::NO_CONTENT)
}

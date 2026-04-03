use crate::{event::now_ms, pane::Pane, state::AppState};
use axum::{extract::State, Json};
use axum::extract::Query;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Summary of an active pane.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct PaneInfo {
    /// Unique pane identifier
    pub id: Uuid,
    /// Human-readable label, if provided at creation
    pub name: Option<String>,
    /// Process group tag (e.g. domain name)
    pub group: Option<String>,
    pub cols: u16,
    pub rows: u16,
    /// True when the shell process has exited
    pub terminated: bool,
    /// Seconds since the last pane activity (input or PTY output)
    pub idle_seconds: u64,
}

impl PaneInfo {
    pub fn from_pane(pane: &Pane) -> Self {
        let last = pane.last_activity_ms.load(Ordering::Relaxed);
        let now = now_ms();
        let idle_ms = now.saturating_sub(last);
        let size = pane.size();
        PaneInfo {
            id: pane.id,
            name: pane.name.clone(),
            group: pane.group(),
            cols: size.cols,
            rows: size.rows,
            terminated: pane.terminated.load(Ordering::Relaxed),
            idle_seconds: idle_ms / 1000,
        }
    }
}

pub fn list_pane_infos(state: &AppState) -> Vec<PaneInfo> {
    state
        .panes
        .iter()
        .map(|entry| PaneInfo::from_pane(entry.value()))
        .collect()
}

#[derive(Deserialize, IntoParams)]
pub struct ListPanesQuery {
    /// Filter panes by group name
    pub group: Option<String>,
}

/// List all active panes.
#[utoipa::path(
    get,
    path = "/panes",
    params(ListPanesQuery),
    responses(
        (status = 200, description = "Array of active panes", body = Vec<PaneInfo>),
    )
)]
pub async fn list_panes_handler(
    State(state): State<AppState>,
    Query(query): Query<ListPanesQuery>,
) -> Json<Vec<PaneInfo>> {
    let mut panes = list_pane_infos(&state);

    if let Some(ref group) = query.group {
        panes.retain(|p| p.group.as_deref() == Some(group.as_str()));
    }

    Json(panes)
}

/// List all distinct group names across active panes.
#[utoipa::path(
    get,
    path = "/groups",
    responses(
        (status = 200, description = "Array of distinct group names", body = Vec<String>),
    )
)]
pub async fn list_groups_handler(State(state): State<AppState>) -> Json<Vec<String>> {
    let groups: BTreeSet<String> = state
        .panes
        .iter()
        .filter_map(|entry| entry.value().group())
        .collect();

    Json(groups.into_iter().collect())
}

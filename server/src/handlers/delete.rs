use crate::{error::AppError, pane_lifecycle::PaneLifecycleEvent, state::AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
};

/// Kill and remove a pane.
///
/// Sends SIGKILL to the shell process and removes the pane from the active set.
/// Any connected WebSocket subscribers will receive a `Closed` error.
#[utoipa::path(
    delete,
    path = "/panes/{id}",
    params(
        ("id" = String, Path, description = "Pane ID or unique pane name"),
    ),
    responses(
        (status = 204, description = "Pane killed and removed"),
        (status = 400, description = "Pane name is ambiguous"),
        (status = 404, description = "Pane not found"),
    )
)]
pub async fn delete_pane_handler(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, AppError> {
    let (id, pane) = state.remove_pane(&id)?;

    // SIGKILL: cannot be caught or ignored; ensures bash exits even in
    // interactive mode (which ignores SIGTERM). When bash exits, the slave
    // PTY fd closes, which unblocks the spawn_blocking reader with EIO.
    if let Err(error) = pane.kill_process(libc::SIGKILL) {
        state.panes.insert(id, pane);
        return Err(AppError::Internal(error));
    }

    let _ = state
        .pane_lifecycle_tx
        .send(PaneLifecycleEvent::Deleted { id });

    // Take the child out of the Option (synchronous std::sync::Mutex — no await)
    // then drop it on a blocking thread, because Child::drop calls waitpid().
    let child = pane.take_child();
    if let Some(child) = child {
        tokio::task::spawn_blocking(move || drop(child));
    }

    Ok(StatusCode::NO_CONTENT)
    // pane (sans child) is dropped here — fast: just closes PTY fds
}

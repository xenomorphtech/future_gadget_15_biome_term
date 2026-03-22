use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use uuid::Uuid;

/// Kill and remove a pane.
///
/// Sends SIGKILL to the shell process and removes the pane from the active set.
/// Any connected WebSocket subscribers will receive a `Closed` error.
#[utoipa::path(
    delete,
    path = "/panes/{id}",
    params(
        ("id" = Uuid, Path, description = "Pane ID"),
    ),
    responses(
        (status = 204, description = "Pane killed and removed"),
        (status = 404, description = "Pane not found"),
    )
)]
pub async fn delete_pane_handler(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<StatusCode, AppError> {
    let (_, pane) = state
        .panes
        .remove(&id)
        .ok_or_else(|| AppError::NotFound(format!("pane {id} not found")))?;

    // SIGKILL: cannot be caught or ignored; ensures bash exits even in
    // interactive mode (which ignores SIGTERM). When bash exits, the slave
    // PTY fd closes, which unblocks the spawn_blocking reader with EIO.
    if let Some(pid) = pane.child_pid {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }

    // Take the child out of the Option (synchronous std::sync::Mutex — no await)
    // then drop it on a blocking thread, because Child::drop calls waitpid().
    let child = pane.child.lock().unwrap().take();
    if let Some(child) = child {
        tokio::task::spawn_blocking(move || drop(child));
    }

    Ok(StatusCode::NO_CONTENT)
    // pane (sans child) is dropped here — fast: just closes PTY fds
}

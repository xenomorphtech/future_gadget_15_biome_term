use crate::{
    error::AppError, handlers::list::list_pane_infos, pane_lifecycle::PaneLifecycleEvent,
    state::AppState,
};
use axum::{
    extract::ws::{Message, WebSocket},
    extract::{State, WebSocketUpgrade},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;

pub async fn ws_pane_lifecycle_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    Ok(ws.on_upgrade(move |socket| handle_ws(socket, state)))
}

async fn handle_ws(socket: WebSocket, state: AppState) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let mut lifecycle_rx = state.pane_lifecycle_tx.subscribe();

    if let Err(error) = send_snapshot(&mut ws_tx, &state).await {
        eprintln!("Failed to send pane lifecycle snapshot: {error}");
        return;
    }

    loop {
        tokio::select! {
            result = lifecycle_rx.recv() => {
                match result {
                    Ok(event) => {
                        if let Err(error) = send_event(&mut ws_tx, &event).await {
                            eprintln!("Failed to send pane lifecycle event: {error}");
                            break;
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        eprintln!("Pane lifecycle WS client lagged by {n} events");
                        if let Err(error) = send_snapshot(&mut ws_tx, &state).await {
                            eprintln!("Failed to send pane lifecycle resync snapshot: {error}");
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            msg = ws_rx.next() => {
                match msg {
                    None => break,
                    Some(Err(error)) => {
                        eprintln!("Pane lifecycle WS receive error: {error}");
                        break;
                    }
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Ping(payload))) => {
                        if let Err(error) = ws_tx.send(Message::Pong(payload)).await {
                            eprintln!("Failed to send pane lifecycle WS pong: {error}");
                            break;
                        }
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

async fn send_snapshot(
    ws_tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
) -> Result<(), String> {
    let snapshot = PaneLifecycleEvent::Snapshot {
        panes: list_pane_infos(state),
    };
    send_event(ws_tx, &snapshot).await
}

async fn send_event(
    ws_tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    event: &PaneLifecycleEvent,
) -> Result<(), String> {
    let payload = serde_json::to_string(event)
        .map_err(|error| format!("failed to serialize lifecycle event: {error}"))?;

    ws_tx
        .send(Message::Text(payload.into()))
        .await
        .map_err(|error| format!("failed to write lifecycle websocket frame: {error}"))
}

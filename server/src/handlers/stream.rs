use crate::{error::AppError, pane::Pane, state::AppState};
use axum::{
    extract::{Path, State, WebSocketUpgrade},
    response::Response,
};
use axum::extract::ws::{Message, WebSocket};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

pub async fn ws_stream_handler(
    Path(id): Path<Uuid>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    // Clone Arc immediately — never hold DashMap Ref across .await
    let pane = state
        .panes
        .get(&id)
        .map(|r| r.clone())
        .ok_or_else(|| AppError::NotFound(format!("pane {id} not found")))?;

    Ok(ws.on_upgrade(move |socket| handle_ws(socket, pane)))
}

async fn handle_ws(socket: WebSocket, pane: Arc<Pane>) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Subscribe before reading history to avoid race condition
    let mut broadcast_rx = pane.broadcast_tx.subscribe();

    // Send all historical events first
    {
        let log = pane.event_log.read().await;
        for event in log.since(0) {
            let msg = json!({
                "seq": event.seq,
                "timestamp_ms": event.timestamp_ms,
                "data": STANDARD.encode(&event.data),
            });
            if ws_tx
                .send(Message::Text(msg.to_string().into()))
                .await
                .is_err()
            {
                return;
            }
        }
    }

    // Forward new broadcast events, also drain client messages (e.g. pings)
    loop {
        tokio::select! {
            result = broadcast_rx.recv() => {
                match result {
                    Ok(event) => {
                        let msg = json!({
                            "seq": event.seq,
                            "timestamp_ms": event.timestamp_ms,
                            "data": STANDARD.encode(&event.data),
                        });
                        if ws_tx
                            .send(Message::Text(msg.to_string().into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        eprintln!("WS client lagged by {n} events");
                        // Continue — next recv() will catch up
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            msg = ws_rx.next() => {
                match msg {
                    None | Some(Err(_)) => break,
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(_)) => {} // ignore pings/text from client
                }
            }
        }
    }
}

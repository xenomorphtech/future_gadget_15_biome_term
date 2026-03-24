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

    let snapshot = PaneLifecycleEvent::Snapshot {
        panes: list_pane_infos(&state),
    };

    if send_event(&mut ws_tx, &snapshot).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            result = lifecycle_rx.recv() => {
                match result {
                    Ok(event) => {
                        if send_event(&mut ws_tx, &event).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        eprintln!("Pane lifecycle WS client lagged by {n} events");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            msg = ws_rx.next() => {
                match msg {
                    None | Some(Err(_)) => break,
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

async fn send_event(
    ws_tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    event: &PaneLifecycleEvent,
) -> Result<(), ()> {
    let payload = serde_json::to_string(event).map_err(|_| ())?;

    ws_tx
        .send(Message::Text(payload.into()))
        .await
        .map_err(|_| ())
}

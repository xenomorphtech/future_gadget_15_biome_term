use crate::{
    error::AppError,
    event::{now_ms, Event},
    pane::Pane,
    state::AppState,
};
use axum::extract::ws::{Message, WebSocket};
use axum::{
    extract::{Path, Query, State, WebSocketUpgrade},
    response::Response,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum StreamFormat {
    #[default]
    Raw,
    ScreenDiff,
}

#[derive(Debug, Default, Deserialize)]
pub struct StreamQuery {
    pub format: Option<String>,
}

impl StreamQuery {
    fn stream_format(&self) -> Result<StreamFormat, AppError> {
        match self.format.as_deref() {
            None | Some("raw") => Ok(StreamFormat::Raw),
            Some("screen_diff") => Ok(StreamFormat::ScreenDiff),
            Some(other) => Err(AppError::BadRequest(format!(
                "invalid stream format {other}; expected `raw` or `screen_diff`"
            ))),
        }
    }
}

struct ScreenSnapshot {
    seq: u64,
    screen: vt100::Screen,
}

/// Subscribe to live PTY output for a pane.
///
/// Upgrades the connection to a WebSocket. Historical events are sent first
/// (subscribe before reading history avoids a race), then new events are
/// forwarded in real time.
///
/// **Frame format** (text, JSON):
/// ```json
/// { "seq": 42, "timestamp_ms": 1700000000000, "data": "<base64>" }
/// ```
/// `data` is base64-encoded raw PTY output bytes. If the subscriber falls
/// behind, the server injects a synthetic recovery frame using the same shape
/// plus `"resync": true`; applying its bytes restores the full screen state.
///
/// Pass `?format=screen_diff` for a native-friendly mode that replays raw
/// history once, sends a full-screen resync frame, then streams `vt100`
/// framebuffer diffs instead of original PTY bytes.
#[utoipa::path(
    get,
    path = "/panes/{id}/stream",
    params(
        ("id" = Uuid, Path, description = "Pane ID"),
        ("format" = Option<String>, Query, description = "Stream format: `raw` (default) or `screen_diff`"),
    ),
    responses(
        (status = 101, description = "WebSocket upgrade — streams `{seq, timestamp_ms, data}` JSON frames"),
        (status = 400, description = "Invalid stream format"),
        (status = 404, description = "Pane not found"),
    )
)]
pub async fn ws_stream_handler(
    Path(id): Path<Uuid>,
    Query(query): Query<StreamQuery>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let pane = state
        .panes
        .get(&id)
        .map(|r| r.clone())
        .ok_or_else(|| AppError::NotFound(format!("pane {id} not found")))?;
    let stream_format = query.stream_format()?;

    Ok(ws.on_upgrade(move |socket| handle_ws(socket, pane, stream_format)))
}

async fn handle_ws(socket: WebSocket, pane: Arc<Pane>, stream_format: StreamFormat) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Subscribe before reading history to avoid race condition.
    let mut broadcast_rx = pane.broadcast_tx.subscribe();
    let mut last_sent_seq = 0;
    let mut diff_baseline = None;

    let history = {
        let log = pane.event_log.read().await;
        log.since(0)
    };
    for event in history {
        last_sent_seq = event.seq;
        if let Err(error) = send_output_event(&mut ws_tx, &event, false).await {
            eprintln!("Failed to send pane stream history event: {error}");
            return;
        }
    }

    if stream_format == StreamFormat::ScreenDiff {
        match send_screen_resync(&mut ws_tx, &pane).await {
            Ok(snapshot) => {
                last_sent_seq = last_sent_seq.max(snapshot.seq);
                diff_baseline = Some(snapshot.screen);
            }
            Err(error) => {
                eprintln!("Failed to send pane stream initial resync: {error}");
                return;
            }
        }
    }

    loop {
        tokio::select! {
            result = broadcast_rx.recv() => {
                match result {
                    Ok(event) => {
                        if event.seq <= last_sent_seq {
                            continue;
                        }

                        let sent_seq = match stream_format {
                            StreamFormat::Raw => {
                                if let Err(error) = send_output_event(&mut ws_tx, event.as_ref(), false).await {
                                    eprintln!("Failed to send pane stream event: {error}");
                                    break;
                                }
                                event.seq
                            }
                            StreamFormat::ScreenDiff => {
                                let Some(prev_screen) = diff_baseline.as_mut() else {
                                    eprintln!("Pane stream screen_diff mode lost its baseline");
                                    break;
                                };
                                match send_screen_diff(&mut ws_tx, &pane, prev_screen, event.seq).await {
                                    Ok(seq) => seq,
                                    Err(error) => {
                                        eprintln!("Failed to send pane stream diff: {error}");
                                        break;
                                    }
                                }
                            }
                        };

                        last_sent_seq = last_sent_seq.max(sent_seq);
                    }
                    Err(RecvError::Lagged(n)) => {
                        eprintln!("WS client lagged by {n} events, sending screen resync frame");
                        match send_screen_resync(&mut ws_tx, &pane).await {
                            Ok(snapshot) => {
                                last_sent_seq = last_sent_seq.max(snapshot.seq);
                                if let Some(prev_screen) = diff_baseline.as_mut() {
                                    *prev_screen = snapshot.screen;
                                }
                            }
                            Err(error) => {
                                eprintln!("Failed to send pane stream resync: {error}");
                                break;
                            }
                        }
                        // Skip any buffered events older than the resync snapshot we just sent.
                        broadcast_rx = pane.broadcast_tx.subscribe();
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            msg = ws_rx.next() => {
                match msg {
                    None => break,
                    Some(Err(error)) => {
                        eprintln!("Pane stream WS receive error: {error}");
                        break;
                    }
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Ping(payload))) => {
                        if let Err(error) = ws_tx.send(Message::Pong(payload)).await {
                            eprintln!("Failed to send pane stream WS pong: {error}");
                            break;
                        }
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

async fn send_output_event(
    ws_tx: &mut SplitSink<WebSocket, Message>,
    event: &Event,
    resync: bool,
) -> Result<(), String> {
    let payload = stream_event_payload(event.seq, event.timestamp_ms, &event.data, resync);
    send_stream_message(ws_tx, payload).await
}

async fn send_stream_message(
    ws_tx: &mut SplitSink<WebSocket, Message>,
    payload: serde_json::Value,
) -> Result<(), String> {
    ws_tx
        .send(Message::Text(payload.to_string().into()))
        .await
        .map_err(|error| format!("failed to write stream websocket frame: {error}"))
}

fn stream_event_payload(
    seq: u64,
    timestamp_ms: u64,
    data: &[u8],
    resync: bool,
) -> serde_json::Value {
    let mut payload = json!({
        "seq": seq,
        "timestamp_ms": timestamp_ms,
        "data": STANDARD.encode(data),
    });
    if resync {
        payload["resync"] = serde_json::Value::Bool(true);
    }
    payload
}

async fn current_screen_snapshot(pane: &Pane) -> ScreenSnapshot {
    // Read the event log seq first so the screen snapshot is guaranteed to
    // represent at least this sequence number; any newer state will simply
    // make subsequent diffs empty until the broadcast cursor catches up.
    let seq = {
        let log = pane.event_log.read().await;
        log.latest_seq().unwrap_or(0)
    };
    let screen = {
        let parser = pane.parser.read().await;
        parser.screen().clone()
    };

    ScreenSnapshot { seq, screen }
}

async fn send_screen_resync(
    ws_tx: &mut SplitSink<WebSocket, Message>,
    pane: &Pane,
) -> Result<ScreenSnapshot, String> {
    let snapshot = current_screen_snapshot(pane).await;
    let state = snapshot.screen.state_formatted();
    let payload = stream_event_payload(snapshot.seq, now_ms(), &state, true);
    send_stream_message(ws_tx, payload).await?;
    Ok(snapshot)
}

async fn send_screen_diff(
    ws_tx: &mut SplitSink<WebSocket, Message>,
    pane: &Pane,
    prev_screen: &mut vt100::Screen,
    seq: u64,
) -> Result<u64, String> {
    let current_screen = {
        let parser = pane.parser.read().await;
        parser.screen().clone()
    };
    let diff = current_screen.state_diff(prev_screen);
    *prev_screen = current_screen;

    if diff.is_empty() {
        return Ok(seq);
    }

    let payload = stream_event_payload(seq, now_ms(), &diff, false);
    send_stream_message(ws_tx, payload).await?;
    Ok(seq)
}

#[cfg(test)]
async fn lagged_resync_payload(pane: &Pane) -> serde_json::Value {
    let snapshot = current_screen_snapshot(pane).await;
    let state = snapshot.screen.state_formatted();

    stream_event_payload(snapshot.seq, now_ms(), &state, true)
}

#[cfg(test)]
mod tests {
    use super::{lagged_resync_payload, stream_event_payload, StreamFormat, StreamQuery};
    use crate::error::AppError;
    use base64::{engine::general_purpose::STANDARD, Engine};

    fn cleanup_test_pane(pane: &crate::pane::Pane) {
        if let Some(pid) = pane.child_pid {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
        if let Ok(mut guard) = pane.child.lock() {
            drop(guard.take());
        }
    }

    #[test]
    fn stream_event_payload_marks_resync_frames_without_changing_the_event_shape() {
        let payload = stream_event_payload(42, 7, b"\x1b[2J", true);

        assert_eq!(payload["seq"].as_u64(), Some(42));
        assert_eq!(payload["timestamp_ms"].as_u64(), Some(7));
        assert_eq!(
            STANDARD.decode(payload["data"].as_str().unwrap()).unwrap(),
            b"\x1b[2J"
        );
        assert_eq!(payload["resync"].as_bool(), Some(true));
    }

    #[tokio::test]
    async fn lagged_resync_payload_reproduces_the_latest_screen_state() {
        let pane = crate::pane::create_pane(
            crate::pane::PaneSize { cols: 80, rows: 24 },
            Some("/bin/true".to_string()),
            None,
            None,
            10_000,
        )
        .expect("test pane should be created");

        {
            let mut parser = pane.parser.write().await;
            parser.process(b"\x1b[2J\x1b[Hhello");
            parser.process(b"\x1b[2;4Hworld");
        }

        {
            let mut log = pane.event_log.write().await;
            log.push(b"first".to_vec());
            log.push(b"second".to_vec());
        }

        let payload = lagged_resync_payload(&pane).await;
        assert_eq!(payload["resync"].as_bool(), Some(true));
        assert_eq!(payload["seq"].as_u64(), Some(2));

        let mut stale_parser = vt100::Parser::new(24, 80, 0);
        stale_parser.process(b"stale");
        stale_parser.process(
            &STANDARD
                .decode(
                    payload["data"]
                        .as_str()
                        .expect("resync payload should include data"),
                )
                .expect("resync payload should be valid base64"),
        );

        let parser = pane.parser.read().await;
        assert_eq!(stale_parser.screen().contents(), parser.screen().contents());
        assert_eq!(
            stale_parser.screen().cursor_position(),
            parser.screen().cursor_position()
        );

        cleanup_test_pane(&pane);
    }

    #[tokio::test]
    async fn lagged_resync_payload_uses_seq_zero_when_no_events_have_been_logged() {
        let pane = crate::pane::create_pane(
            crate::pane::PaneSize { cols: 80, rows: 24 },
            Some("/bin/true".to_string()),
            None,
            None,
            10_000,
        )
        .expect("test pane should be created");

        {
            let mut parser = pane.parser.write().await;
            parser.process(b"prompt$ ");
        }

        let payload = lagged_resync_payload(&pane).await;
        assert_eq!(payload["seq"].as_u64(), Some(0));
        assert_eq!(payload["resync"].as_bool(), Some(true));

        cleanup_test_pane(&pane);
    }

    #[test]
    fn stream_query_accepts_raw_and_screen_diff_formats() {
        assert_eq!(
            StreamQuery::default().stream_format().unwrap(),
            StreamFormat::Raw
        );
        assert_eq!(
            StreamQuery {
                format: Some("screen_diff".to_string())
            }
            .stream_format()
            .unwrap(),
            StreamFormat::ScreenDiff
        );
    }

    #[test]
    fn stream_query_rejects_unknown_formats() {
        let err = StreamQuery {
            format: Some("bogus".to_string()),
        }
        .stream_format()
        .unwrap_err();

        match err {
            AppError::BadRequest(message) => assert!(message.contains("invalid stream format")),
            other => panic!("expected bad request, got {other:?}"),
        }
    }
}

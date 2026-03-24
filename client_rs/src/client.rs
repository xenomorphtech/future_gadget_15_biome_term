use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{stream::BoxStream, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use crate::{
    types::{CreatePaneOptions, Event, LifecycleEvent, PaneInfo, RawEvent, ScreenResponse},
    Error,
};

/// Async HTTP + WebSocket client for the biome_term server.
pub struct BiomeTermClient {
    http: reqwest::Client,
    base_url: String,
}

impl BiomeTermClient {
    /// Create a new client targeting `base_url` (e.g. `"http://localhost:3000"`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }

    // ── REST API ──────────────────────────────────────────────────────────────

    /// Create a new terminal pane.
    pub async fn create_pane(&self, opts: CreatePaneOptions) -> Result<PaneInfo, Error> {
        let resp = self
            .http
            .post(format!("{}/panes", self.base_url))
            .json(&opts)
            .send()
            .await?;
        Ok(self.check_status(resp).await?.json().await?)
    }

    /// List all panes (including terminated ones).
    pub async fn list_panes(&self) -> Result<Vec<PaneInfo>, Error> {
        let resp = self
            .http
            .get(format!("{}/panes", self.base_url))
            .send()
            .await?;
        Ok(self.check_status(resp).await?.json().await?)
    }

    /// Kill and remove a pane.
    pub async fn delete_pane(&self, id: &str) -> Result<(), Error> {
        let resp = self
            .http
            .delete(format!("{}/panes/{}", self.base_url, id))
            .send()
            .await?;
        self.check_status(resp).await?;
        Ok(())
    }

    /// Send raw bytes to the pane's PTY stdin.
    pub async fn send_input(&self, id: &str, data: &[u8]) -> Result<(), Error> {
        let body = serde_json::json!({ "data": STANDARD.encode(data) });
        let resp = self
            .http
            .post(format!("{}/panes/{}/input", self.base_url, id))
            .json(&body)
            .send()
            .await?;
        self.check_status(resp).await?;
        Ok(())
    }

    /// Resize the pane's terminal.
    pub async fn resize_pane(&self, id: &str, cols: u16, rows: u16) -> Result<(), Error> {
        let body = serde_json::json!({ "cols": cols, "rows": rows });
        let resp = self
            .http
            .post(format!("{}/panes/{}/resize", self.base_url, id))
            .json(&body)
            .send()
            .await?;
        self.check_status(resp).await?;
        Ok(())
    }

    /// Get the current VT100 screen state.
    pub async fn get_screen(&self, id: &str) -> Result<ScreenResponse, Error> {
        let resp = self
            .http
            .get(format!("{}/panes/{}/screen", self.base_url, id))
            .send()
            .await?;
        Ok(self.check_status(resp).await?.json().await?)
    }

    /// Fetch event log entries.  Pass `after_seq` to only return events with
    /// `seq > after_seq` (use `0` or `None` for the full log).
    pub async fn get_events(
        &self,
        id: &str,
        after_seq: Option<u64>,
    ) -> Result<Vec<Event>, Error> {
        let url = match after_seq {
            Some(seq) => format!("{}/panes/{}/events?after={}", self.base_url, id, seq),
            None => format!("{}/panes/{}/events", self.base_url, id),
        };
        let resp = self.http.get(&url).send().await?;
        let raw: Vec<RawEvent> = self.check_status(resp).await?.json().await?;
        raw.into_iter()
            .map(|r| {
                Ok(Event {
                    seq: r.seq,
                    timestamp_ms: r.timestamp_ms,
                    data: STANDARD.decode(&r.data)?,
                })
            })
            .collect()
    }

    // ── WebSocket streams ─────────────────────────────────────────────────────

    /// Stream PTY output events from a pane via WebSocket.
    ///
    /// The server replays historical events first, then streams new ones live.
    /// Returns a `Stream` of decoded [`Event`]s.
    pub async fn stream_pane(
        &self,
        id: &str,
    ) -> Result<BoxStream<'static, Result<Event, Error>>, Error> {
        let url = format!("{}/panes/{}/stream", self.ws_base(), id);
        let (ws, _) = tokio_tungstenite::connect_async(url).await?;
        let stream = ws.filter_map(|msg| async move {
            match msg {
                Ok(Message::Text(txt)) => {
                    let raw: RawEvent = match serde_json::from_str(&txt) {
                        Ok(r) => r,
                        Err(e) => return Some(Err(Error::Json(e))),
                    };
                    match STANDARD.decode(&raw.data) {
                        Ok(data) => Some(Ok(Event {
                            seq: raw.seq,
                            timestamp_ms: raw.timestamp_ms,
                            data,
                        })),
                        Err(e) => Some(Err(Error::Base64(e))),
                    }
                }
                Ok(Message::Close(_)) => None,
                Ok(_) => None,
                Err(e) => Some(Err(Error::WebSocket(e))),
            }
        });
        Ok(Box::pin(stream))
    }

    /// Stream pane lifecycle events (created / deleted) via WebSocket.
    ///
    /// The server sends a `snapshot` message first with all current panes,
    /// then live `created` / `deleted` events.
    pub async fn stream_lifecycle(
        &self,
    ) -> Result<BoxStream<'static, Result<LifecycleEvent, Error>>, Error> {
        let url = format!("{}/panes/lifecycle", self.ws_base());
        let (ws, _) = tokio_tungstenite::connect_async(url).await?;
        let stream = ws.filter_map(|msg| async move {
            match msg {
                Ok(Message::Text(txt)) => {
                    match serde_json::from_str::<LifecycleEvent>(&txt) {
                        Ok(event) => Some(Ok(event)),
                        Err(e) => Some(Err(Error::Json(e))),
                    }
                }
                Ok(Message::Close(_)) => None,
                Ok(_) => None,
                Err(e) => Some(Err(Error::WebSocket(e))),
            }
        });
        Ok(Box::pin(stream))
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn ws_base(&self) -> String {
        self.base_url
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1)
    }

    async fn check_status(&self, resp: reqwest::Response) -> Result<reqwest::Response, Error> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::NOT_FOUND {
            Err(Error::NotFound(body))
        } else {
            Err(Error::Server(format!("{status}: {body}")))
        }
    }
}

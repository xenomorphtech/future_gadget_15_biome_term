use serde::{Deserialize, Serialize};

/// Options for creating a new pane.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CreatePaneOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Info about a pane returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneInfo {
    pub id: String,
    pub name: Option<String>,
    pub cols: u16,
    pub rows: u16,
    #[serde(default)]
    pub terminated: bool,
}

/// Current VT100 screen state of a pane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenResponse {
    pub rows: Vec<String>,
    pub cursor_col: u16,
    pub cursor_row: u16,
    pub num_cols: u16,
    pub num_rows: u16,
}

/// A single PTY output event.  `data` contains raw PTY bytes, decoded from
/// the base64 the server sends over the wire.
#[derive(Debug, Clone)]
pub struct Event {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub data: Vec<u8>,
}

/// Deserialization helper — the server encodes `data` as base64.
#[derive(Deserialize)]
pub(crate) struct RawEvent {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub data: String,
}

/// A pane lifecycle event from the `/panes/lifecycle` WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LifecycleEvent {
    Snapshot { panes: Vec<PaneInfo> },
    Created { pane: PaneInfo },
    Deleted { id: String },
}

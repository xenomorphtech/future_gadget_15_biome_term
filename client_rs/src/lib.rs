mod client;
pub mod types;

pub use client::BiomeTermClient;
pub use types::{CreatePaneOptions, Event, LifecycleEvent, PaneInfo, ScreenResponse};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Server error: {0}")]
    Server(String),
}

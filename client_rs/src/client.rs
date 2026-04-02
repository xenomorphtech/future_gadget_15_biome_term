use std::{io::BufReader, sync::Arc};

use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{stream::BoxStream, StreamExt};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use rustls::{
    client::{
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        verify_server_cert_signed_by_trust_anchor, WebPkiServerVerifier,
    },
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use tokio_tungstenite::{
    connect_async_tls_with_config,
    tungstenite::{client::IntoClientRequest, Message},
    Connector,
};

use crate::{
    types::{CreatePaneOptions, Event, LifecycleEvent, PaneInfo, RawEvent, ScreenResponse},
    Error,
};

/// Async HTTP + WebSocket client for the biome_term server.
pub struct BiomeTermClient {
    http: reqwest::Client,
    base_url: String,
    auth_header: Option<HeaderValue>,
    api_key_header: Option<HeaderValue>,
    ws_connector: Connector,
}

/// Builder for [`BiomeTermClient`] transport settings.
pub struct BiomeTermClientBuilder {
    base_url: String,
    api_key: Option<String>,
    root_certificates_pem: Vec<Vec<u8>>,
    accept_invalid_certs: bool,
    accept_invalid_hostnames: bool,
}

impl BiomeTermClient {
    /// Create a new client targeting `base_url` (e.g. `"http://localhost:3021"`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::builder(base_url)
            .build()
            .expect("default biome_term client configuration should be valid")
    }

    /// Create a builder for custom auth or TLS settings.
    pub fn builder(base_url: impl Into<String>) -> BiomeTermClientBuilder {
        BiomeTermClientBuilder {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: None,
            root_certificates_pem: Vec::new(),
            accept_invalid_certs: false,
            accept_invalid_hostnames: false,
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
    pub async fn get_events(&self, id: &str, after_seq: Option<u64>) -> Result<Vec<Event>, Error> {
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
        let request = self.ws_request(url)?;
        let (ws, _) =
            connect_async_tls_with_config(request, None, false, Some(self.ws_connector.clone()))
                .await?;
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
        let request = self.ws_request(url)?;
        let (ws, _) =
            connect_async_tls_with_config(request, None, false, Some(self.ws_connector.clone()))
                .await?;
        let stream = ws.filter_map(|msg| async move {
            match msg {
                Ok(Message::Text(txt)) => match serde_json::from_str::<LifecycleEvent>(&txt) {
                    Ok(event) => Some(Ok(event)),
                    Err(e) => Some(Err(Error::Json(e))),
                },
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

    fn ws_request(
        &self,
        url: String,
    ) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, Error> {
        let mut request = url.into_client_request()?;
        if let Some(value) = self.auth_header.clone() {
            request.headers_mut().insert(AUTHORIZATION, value);
        }
        if let Some(value) = self.api_key_header.clone() {
            request.headers_mut().insert("x-api-key", value);
        }
        Ok(request)
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

impl BiomeTermClientBuilder {
    /// Send `Authorization: Bearer <api_key>` on HTTP and WebSocket requests.
    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = normalize_optional_string(Some(api_key.into()));
        self
    }

    /// Trust one or more PEM-encoded root certificates for HTTPS and WSS.
    pub fn add_root_certificate_pem(mut self, pem: impl Into<Vec<u8>>) -> Self {
        self.root_certificates_pem.push(pem.into());
        self
    }

    /// Accept invalid or self-signed certificates.
    pub fn danger_accept_invalid_certs(mut self, accept_invalid_certs: bool) -> Self {
        self.accept_invalid_certs = accept_invalid_certs;
        self
    }

    /// Skip TLS hostname validation.
    pub fn danger_accept_invalid_hostnames(mut self, accept_invalid_hostnames: bool) -> Self {
        self.accept_invalid_hostnames = accept_invalid_hostnames;
        self
    }

    /// Build the configured client.
    pub fn build(self) -> Result<BiomeTermClient, Error> {
        let auth_header = self.authorization_header()?;
        let api_key_header = self.api_key_header()?;
        let default_headers = default_headers(auth_header.as_ref(), api_key_header.as_ref());
        let ws_connector = self.ws_connector()?;

        let mut http = reqwest::Client::builder().default_headers(default_headers);
        if self.accept_invalid_certs {
            http = http.danger_accept_invalid_certs(true);
        }
        if self.accept_invalid_hostnames {
            http = http.danger_accept_invalid_hostnames(true);
        }
        for pem in &self.root_certificates_pem {
            for cert in reqwest::Certificate::from_pem_bundle(pem)? {
                http = http.add_root_certificate(cert);
            }
        }

        Ok(BiomeTermClient {
            http: http.build()?,
            base_url: self.base_url,
            auth_header,
            api_key_header,
            ws_connector,
        })
    }

    fn authorization_header(&self) -> Result<Option<HeaderValue>, Error> {
        self.api_key
            .as_ref()
            .map(|api_key| HeaderValue::from_str(&format!("Bearer {api_key}")))
            .transpose()
            .map_err(Error::from)
    }

    fn api_key_header(&self) -> Result<Option<HeaderValue>, Error> {
        self.api_key
            .as_ref()
            .map(|api_key| HeaderValue::from_str(api_key))
            .transpose()
            .map_err(Error::from)
    }

    fn ws_connector(&self) -> Result<Connector, Error> {
        if !self.base_url.starts_with("https://") {
            return Ok(Connector::Plain);
        }

        let roots = Arc::new(self.ws_root_store()?);
        let client_config = if self.accept_invalid_certs {
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAllTlsVerifier))
                .with_no_client_auth()
        } else if self.accept_invalid_hostnames {
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(HostnameIgnoringTlsVerifier::new(
                    roots.clone(),
                )?))
                .with_no_client_auth()
        } else {
            rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth()
        };

        Ok(Connector::Rustls(Arc::new(client_config)))
    }

    fn ws_root_store(&self) -> Result<RootCertStore, Error> {
        let mut roots = RootCertStore::empty();
        let rustls_native_certs::CertificateResult { certs, .. } =
            rustls_native_certs::load_native_certs();
        roots.add_parsable_certificates(certs);

        for pem in &self.root_certificates_pem {
            let certs = rustls_pem_certificates(pem)?;
            roots.add_parsable_certificates(certs);
        }

        Ok(roots)
    }
}

fn default_headers(
    auth_header: Option<&HeaderValue>,
    api_key_header: Option<&HeaderValue>,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(value) = auth_header {
        headers.insert(AUTHORIZATION, value.clone());
    }
    if let Some(value) = api_key_header {
        headers.insert("x-api-key", value.clone());
    }
    headers
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else if trimmed.len() == value.len() {
            Some(value)
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn rustls_pem_certificates(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, Error> {
    rustls_pemfile::certs(&mut BufReader::new(pem))
        .collect::<Result<Vec<_>, _>>()
        .map_err(Error::from)
}

fn tls_supported_schemes() -> Vec<SignatureScheme> {
    rustls::crypto::ring::default_provider()
        .signature_verification_algorithms
        .supported_schemes()
}

#[derive(Debug)]
struct AcceptAllTlsVerifier;

impl ServerCertVerifier for AcceptAllTlsVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        tls_supported_schemes()
    }
}

#[derive(Debug)]
struct HostnameIgnoringTlsVerifier {
    roots: Arc<RootCertStore>,
    inner: Arc<WebPkiServerVerifier>,
}

impl HostnameIgnoringTlsVerifier {
    fn new(roots: Arc<RootCertStore>) -> Result<Self, Error> {
        let inner = WebPkiServerVerifier::builder(roots.clone())
            .build()
            .map_err(|err| Error::Server(format!("failed to build TLS verifier: {err}")))?;
        Ok(Self { roots, inner })
    }
}

impl ServerCertVerifier for HostnameIgnoringTlsVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let cert = rustls::server::ParsedCertificate::try_from(end_entity)?;
        verify_server_cert_signed_by_trust_anchor(
            &cert,
            &self.roots,
            intermediates,
            now,
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .all,
        )?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }

    fn root_hint_subjects(&self) -> Option<&[rustls::DistinguishedName]> {
        self.inner.root_hint_subjects()
    }
}

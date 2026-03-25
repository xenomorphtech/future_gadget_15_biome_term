use std::{env, net::ToSocketAddrs, time::Duration};

use axum_server::{tls_rustls::RustlsConfig, Handle};
use terminal_server::{build_router, state::AppState};
use tokio::net::TcpListener;
use tokio::sync::watch;

#[tokio::main]
async fn main() {
    let state = AppState::from_env();
    let router = build_router(state.clone());
    let http_config = HttpConfig::from_env();
    let tls_config = TlsConfig::from_env(http_config.port);
    let shutdown = spawn_shutdown_signal();

    match tls_config {
        Some(tls) => {
            let http_server = serve_http(
                router.clone(),
                &http_config.listen_addr,
                shutdown.subscribe(),
            );
            let https_server = serve_https(router, tls, shutdown.subscribe());
            tokio::join!(http_server, https_server);
        }
        None => serve_http(router, &http_config.listen_addr, shutdown.subscribe()).await,
    }

    // Kill all child processes on shutdown
    println!("Shutting down: killing {} pane(s)", state.panes.len());
    for entry in state.panes.iter() {
        let pane = entry.value();
        if let Some(pid) = pane.child_pid {
            let pid = pid as libc::pid_t;
            if pid > 0 {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
        if let Ok(mut guard) = pane.child.lock() {
            drop(guard.take());
        }
    }
    println!("Shutdown complete");
}

async fn serve_http(router: axum::Router, listen_addr: &str, mut shutdown: watch::Receiver<bool>) {
    let listener = TcpListener::bind(listen_addr)
        .await
        .expect("failed to bind terminal server listen address");

    println!("Terminal server listening on http://{listen_addr}");

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = shutdown.changed().await;
        })
        .await
        .expect("server error");
}

async fn serve_https(router: axum::Router, tls: TlsConfig, mut shutdown: watch::Receiver<bool>) {
    let listen_addr = tls.listen_addr.clone();
    let rustls_config = RustlsConfig::from_pem_file(&tls.cert_path, &tls.key_path)
        .await
        .expect("failed to load BIOME_TLS_CERT / BIOME_TLS_KEY");
    let socket_addr = tokio::net::lookup_host(&listen_addr)
        .await
        .expect("failed to resolve terminal server listen address")
        .next()
        .expect("no listen addresses resolved for terminal server");
    let handle = Handle::new();
    let shutdown_handle = handle.clone();

    tokio::spawn(async move {
        let _ = shutdown.changed().await;
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(30)));
    });

    println!("Terminal server listening on https://{listen_addr}");

    axum_server::bind_rustls(socket_addr, rustls_config)
        .handle(handle)
        .serve(router.into_make_service())
        .await
        .expect("server error");
}

fn spawn_shutdown_signal() -> watch::Sender<bool> {
    let (shutdown_tx, _) = watch::channel(false);
    let signal_tx = shutdown_tx.clone();

    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = signal_tx.send(true);
    });

    shutdown_tx
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    println!("Shutdown signal received");
}

struct HttpConfig {
    listen_addr: String,
    port: u16,
}

impl HttpConfig {
    fn from_env() -> Self {
        let configured_addr =
            env::var("LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
        let port = resolve_port(&configured_addr, "LISTEN_ADDR");

        Self {
            listen_addr: format!("127.0.0.1:{port}"),
            port,
        }
    }
}

struct TlsConfig {
    cert_path: String,
    key_path: String,
    listen_addr: String,
}

impl TlsConfig {
    fn from_env(http_port: u16) -> Option<Self> {
        let cert_path = env::var("BIOME_TLS_CERT")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let key_path = env::var("BIOME_TLS_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty());

        match (cert_path, key_path) {
            (Some(cert_path), Some(key_path)) => {
                let listen_addr = env::var("BIOME_TLS_LISTEN_ADDR")
                    .unwrap_or_else(|_| "0.0.0.0:3443".to_string());
                let tls_port = resolve_port(&listen_addr, "BIOME_TLS_LISTEN_ADDR");

                assert_ne!(
                    tls_port, http_port,
                    "BIOME_TLS_LISTEN_ADDR must use a different port than LISTEN_ADDR"
                );

                Some(Self {
                    cert_path,
                    key_path,
                    listen_addr,
                })
            }
            (Some(_), None) | (None, Some(_)) => {
                eprintln!(
                    "Ignoring partial TLS configuration because BIOME_TLS_CERT and BIOME_TLS_KEY must both be set"
                );
                None
            }
            (None, None) => None,
        }
    }
}

fn resolve_port(listen_addr: &str, env_name: &str) -> u16 {
    listen_addr
        .to_socket_addrs()
        .unwrap_or_else(|_| panic!("failed to resolve {env_name}: {listen_addr}"))
        .next()
        .unwrap_or_else(|| panic!("no socket addresses resolved for {env_name}: {listen_addr}"))
        .port()
}

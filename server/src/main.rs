use std::{
    env, io,
    net::{SocketAddr, ToSocketAddrs},
    time::Duration,
};

use axum_server::{tls_rustls::RustlsConfig, Handle};
use terminal_server::{build_router, state::AppState};
use tokio::net::TcpListener;
use tokio::sync::watch;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Terminal server exited with error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> io::Result<()> {
    raise_nofile_limit();
    let state = AppState::from_env();
    let router = build_router(state.clone());
    let http_config = HttpConfig::from_env()?;
    let tls_config = TlsConfig::from_env(http_config.port)?;
    let shutdown = spawn_shutdown_signal();

    let serve_result = match tls_config {
        Some(tls) => {
            let http_server = serve_http(
                router.clone(),
                &http_config.listen_addr,
                shutdown.subscribe(),
            );
            let https_server = serve_https(router, tls, shutdown.subscribe());
            run_servers(http_server, https_server, &shutdown).await
        }
        None => serve_http(router, &http_config.listen_addr, shutdown.subscribe()).await,
    };

    shutdown_panes(&state);
    serve_result
}

async fn run_servers(
    http_server: impl std::future::Future<Output = io::Result<()>>,
    https_server: impl std::future::Future<Output = io::Result<()>>,
    shutdown: &watch::Sender<bool>,
) -> io::Result<()> {
    tokio::pin!(http_server);
    tokio::pin!(https_server);

    tokio::select! {
        http_result = &mut http_server => {
            if http_result.is_err() {
                let _ = shutdown.send(true);
            }
            let https_result = https_server.await;
            http_result.and(https_result)
        }
        https_result = &mut https_server => {
            if https_result.is_err() {
                let _ = shutdown.send(true);
            }
            let http_result = http_server.await;
            http_result.and(https_result)
        }
    }
}

async fn serve_http(
    router: axum::Router,
    listen_addr: &str,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    let listener = TcpListener::bind(listen_addr).await?;

    println!("Terminal server listening on http://{listen_addr}");

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = shutdown.changed().await;
        })
        .await
}

async fn serve_https(
    router: axum::Router,
    tls: TlsConfig,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    let listen_addr = tls.listen_addr.clone();
    let rustls_config = RustlsConfig::from_pem_file(&tls.cert_path, &tls.key_path).await?;
    let socket_addr =
        resolve_socket_addr(tokio::net::lookup_host(&listen_addr).await?, &listen_addr)?;
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
}

fn spawn_shutdown_signal() -> watch::Sender<bool> {
    let (shutdown_tx, _) = watch::channel(false);
    spawn_ctrl_c_handler(shutdown_tx.clone());
    #[cfg(unix)]
    spawn_sigterm_handler(shutdown_tx.clone());

    shutdown_tx
}

fn spawn_ctrl_c_handler(signal_tx: watch::Sender<bool>) {
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                println!("Shutdown signal received (Ctrl+C)");
                let _ = signal_tx.send(true);
            }
            Err(error) => eprintln!("Failed to install Ctrl+C handler: {error}"),
        }
    });
}

#[cfg(unix)]
fn spawn_sigterm_handler(signal_tx: watch::Sender<bool>) {
    tokio::spawn(async move {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => {
                terminate.recv().await;
                println!("Shutdown signal received (SIGTERM)");
                let _ = signal_tx.send(true);
            }
            Err(error) => eprintln!("Failed to install SIGTERM handler: {error}"),
        }
    });
}

fn shutdown_panes(state: &AppState) {
    println!("Shutting down: killing {} pane(s)", state.panes.len());
    for entry in state.panes.iter() {
        let pane = entry.value();
        if let Err(error) = pane.kill_process(libc::SIGKILL) {
            eprintln!("Failed to kill pane {} during shutdown: {error}", pane.id);
        }
        drop(pane.take_child());
    }
    println!("Shutdown complete");
}

struct HttpConfig {
    listen_addr: String,
    port: u16,
}

impl HttpConfig {
    fn from_env() -> io::Result<Self> {
        let configured_addr =
            env::var("LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:3021".to_string());
        let port = resolve_port(&configured_addr, "LISTEN_ADDR")?;

        Ok(Self {
            listen_addr: format!("127.0.0.1:{port}"),
            port,
        })
    }
}

struct TlsConfig {
    cert_path: String,
    key_path: String,
    listen_addr: String,
}

impl TlsConfig {
    fn from_env(http_port: u16) -> io::Result<Option<Self>> {
        let cert_path = env::var("BIOME_TLS_CERT")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let key_path = env::var("BIOME_TLS_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty());

        match (cert_path, key_path) {
            (Some(cert_path), Some(key_path)) => {
                let listen_addr = env::var("BIOME_TLS_LISTEN_ADDR")
                    .unwrap_or_else(|_| "0.0.0.0:3027".to_string());
                let tls_port = resolve_port(&listen_addr, "BIOME_TLS_LISTEN_ADDR")?;

                if tls_port == http_port {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "BIOME_TLS_LISTEN_ADDR must use a different port than LISTEN_ADDR",
                    ));
                }

                Ok(Some(Self {
                    cert_path,
                    key_path,
                    listen_addr,
                }))
            }
            (Some(_), None) | (None, Some(_)) => {
                eprintln!(
                    "Ignoring partial TLS configuration because BIOME_TLS_CERT and BIOME_TLS_KEY must both be set"
                );
                Ok(None)
            }
            (None, None) => Ok(None),
        }
    }
}

/// Raise the open-file limit (RLIMIT_NOFILE) to the hard limit.
fn raise_nofile_limit() {
    #[cfg(unix)]
    {
        let mut rlim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        unsafe {
            if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) == 0 {
                if rlim.rlim_cur < rlim.rlim_max {
                    let prev = rlim.rlim_cur;
                    rlim.rlim_cur = rlim.rlim_max;
                    if libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) == 0 {
                        println!("Raised open-file limit from {prev} to {}", rlim.rlim_max);
                    } else {
                        eprintln!("Failed to raise open-file limit (setrlimit)");
                    }
                }
            } else {
                eprintln!("Failed to query open-file limit (getrlimit)");
            }
        }
    }
}

fn resolve_port(listen_addr: &str, env_name: &str) -> io::Result<u16> {
    Ok(resolve_socket_addr(
        listen_addr.to_socket_addrs()?,
        &format!("{env_name}: {listen_addr}"),
    )?
    .port())
}

fn resolve_socket_addr(
    mut addrs: impl Iterator<Item = SocketAddr>,
    context: &str,
) -> io::Result<SocketAddr> {
    addrs.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no socket addresses resolved for {context}"),
        )
    })
}

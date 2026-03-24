use std::env;

use terminal_server::{build_router, state::AppState};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let state = AppState::new();
    let router = build_router(state.clone());
    let listen_addr = env::var("LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_string());

    let listener = TcpListener::bind(&listen_addr)
        .await
        .expect("failed to bind terminal server listen address");

    println!("Terminal server listening on http://{listen_addr}");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

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

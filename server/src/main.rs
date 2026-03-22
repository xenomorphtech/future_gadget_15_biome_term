use terminal_server::{build_router, state::AppState};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let state = AppState::new();
    let router = build_router(state);

    let listener = TcpListener::bind("0.0.0.0:3000")
        .await
        .expect("failed to bind to port 3000");

    println!("Terminal server listening on http://0.0.0.0:3000");

    axum::serve(listener, router)
        .await
        .expect("server error");
}

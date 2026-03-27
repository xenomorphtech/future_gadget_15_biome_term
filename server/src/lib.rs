pub mod auth;
pub mod error;
pub mod event;
pub mod handlers;
pub mod openapi;
pub mod pane;
pub mod pane_lifecycle;
pub mod state;

use auth::require_api_key;
use axum::{
    middleware,
    routing::{delete, get, patch, post},
    Json, Router,
};
use handlers::{
    config::{get_config_handler, update_config_handler},
    create::create_pane_handler,
    delete::delete_pane_handler,
    events::get_events_handler,
    input::send_input_handler,
    lifecycle::ws_pane_lifecycle_handler,
    list::list_panes_handler,
    resize::resize_pane_handler,
    screen::get_screen_handler,
    stream::ws_stream_handler,
};
use openapi::ApiDoc;
use state::AppState;
use std::sync::LazyLock;
use utoipa::OpenApi;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/config", get(get_config_handler))
        .route("/config", patch(update_config_handler))
        .route("/panes", post(create_pane_handler))
        .route("/panes", get(list_panes_handler))
        .route("/panes/lifecycle", get(ws_pane_lifecycle_handler))
        .route("/panes/{id}", delete(delete_pane_handler))
        .route("/panes/{id}/input", post(send_input_handler))
        .route("/panes/{id}/resize", post(resize_pane_handler))
        .route("/panes/{id}/screen", get(get_screen_handler))
        .route("/panes/{id}/events", get(get_events_handler))
        .route("/panes/{id}/stream", get(ws_stream_handler))
        .route("/openapi.json", get(openapi_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ))
        .with_state(state)
}

static OPENAPI_JSON: LazyLock<serde_json::Value> = LazyLock::new(|| {
    let json_str = ApiDoc::openapi()
        .to_pretty_json()
        .expect("OpenAPI schema serialization failed");
    serde_json::from_str(&json_str).expect("OpenAPI schema is not valid JSON")
});

async fn openapi_handler() -> Json<serde_json::Value> {
    Json(OPENAPI_JSON.clone())
}

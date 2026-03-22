pub mod error;
pub mod event;
pub mod handlers;
pub mod openapi;
pub mod pane;
pub mod state;

use axum::{
    routing::{delete, get, post},
    Json, Router,
};
use handlers::{
    create::create_pane_handler,
    delete::delete_pane_handler,
    events::get_events_handler,
    input::send_input_handler,
    list::list_panes_handler,
    resize::resize_pane_handler,
    screen::get_screen_handler,
    stream::ws_stream_handler,
};
use openapi::ApiDoc;
use state::AppState;
use utoipa::OpenApi;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/panes", post(create_pane_handler))
        .route("/panes", get(list_panes_handler))
        .route("/panes/{id}", delete(delete_pane_handler))
        .route("/panes/{id}/input", post(send_input_handler))
        .route("/panes/{id}/resize", post(resize_pane_handler))
        .route("/panes/{id}/screen", get(get_screen_handler))
        .route("/panes/{id}/events", get(get_events_handler))
        .route("/panes/{id}/stream", get(ws_stream_handler))
        .route("/openapi.json", get(openapi_handler))
        .with_state(state)
}

async fn openapi_handler() -> Json<serde_json::Value> {
    Json(serde_json::from_str(&ApiDoc::openapi().to_pretty_json().unwrap()).unwrap())
}

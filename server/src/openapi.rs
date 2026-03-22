use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Terminal Server API",
        version = "0.1.0",
        description = "HTTP + WebSocket API for managing persistent PTY sessions backed by a VT100 emulator. \
                       Each pane is an independent shell process; the server maintains authoritative terminal \
                       state so clients can reconnect without losing history."
    ),
    paths(
        crate::handlers::create::create_pane_handler,
        crate::handlers::list::list_panes_handler,
        crate::handlers::delete::delete_pane_handler,
        crate::handlers::input::send_input_handler,
        crate::handlers::resize::resize_pane_handler,
        crate::handlers::screen::get_screen_handler,
        crate::handlers::events::get_events_handler,
        crate::handlers::stream::ws_stream_handler,
    ),
    components(schemas(
        crate::handlers::create::CreatePaneRequest,
        crate::handlers::create::CreatePaneResponse,
        crate::handlers::list::PaneInfo,
        crate::handlers::input::InputRequest,
        crate::handlers::resize::ResizeRequest,
        crate::handlers::screen::ScreenResponse,
        crate::handlers::events::EventResponse,
    ))
)]
pub struct ApiDoc;

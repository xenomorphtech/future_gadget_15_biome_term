use axum::{
    body::Body,
    extract::State,
    http::{
        header::{AUTHORIZATION, WWW_AUTHENTICATE},
        HeaderMap, HeaderValue, Request, StatusCode,
    },
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::state::AppState;

pub async fn require_api_key(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    match state.api_key() {
        None => next.run(request).await,
        Some(expected_key) if request_is_authorized(request.headers(), expected_key) => {
            next.run(request).await
        }
        Some(_) => unauthorized_response(),
    }
}

fn request_is_authorized(headers: &HeaderMap, expected_key: &str) -> bool {
    bearer_token(headers).is_some_and(|token| token == expected_key)
        || api_key_header(headers).is_some_and(|token| token == expected_key)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let mut parts = value.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;

    if parts.next().is_none() && scheme.eq_ignore_ascii_case("Bearer") {
        Some(token)
    } else {
        None
    }
}

fn api_key_header(headers: &HeaderMap) -> Option<&str> {
    headers.get("x-api-key")?.to_str().ok()
}

fn unauthorized_response() -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "unauthorized" })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

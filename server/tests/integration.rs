use axum::http::{
    header::{HeaderValue, AUTHORIZATION},
    StatusCode,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

async fn start_test_server() -> String {
    start_test_server_with_api_key(None).await
}

async fn start_test_server_with_api_key(api_key: Option<&str>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = terminal_server::state::AppState::with_api_key(api_key.map(str::to_string));
    let router = terminal_server::build_router(state);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://127.0.0.1:{}", port)
}

async fn create_pane(client: &reqwest::Client, base: &str) -> String {
    let resp = create_pane_response(client, base, None).await;
    resp["id"].as_str().unwrap().to_string()
}

async fn create_pane_with_name(client: &reqwest::Client, base: &str, name: &str) -> String {
    let resp = create_pane_response(client, base, Some(name)).await;
    resp["id"].as_str().unwrap().to_string()
}

async fn create_pane_response(
    client: &reqwest::Client,
    base: &str,
    name: Option<&str>,
) -> serde_json::Value {
    let mut body = serde_json::json!({ "cols": 80, "rows": 24 });
    if let Some(name) = name {
        body["name"] = serde_json::Value::String(name.to_string());
    }

    create_pane_with_body_response(client, base, body).await
}

async fn create_pane_with_body_response(
    client: &reqwest::Client,
    base: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    client
        .post(format!("{base}/panes"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn get_config(client: &reqwest::Client, base: &str) -> serde_json::Value {
    client
        .get(format!("{base}/config"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn send_input(client: &reqwest::Client, base: &str, id: &str, text: &str) {
    client
        .post(format!("{base}/panes/{id}/input"))
        .json(&serde_json::json!({ "data": STANDARD.encode(text.as_bytes()) }))
        .send()
        .await
        .unwrap();
}

async fn get_screen(client: &reqwest::Client, base: &str, id: &str) -> serde_json::Value {
    client
        .get(format!("{base}/panes/{id}/screen"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn delete_pane(client: &reqwest::Client, base: &str, id: &str) {
    client
        .delete(format!("{base}/panes/{id}"))
        .send()
        .await
        .unwrap();
}

async fn list_panes(client: &reqwest::Client, base: &str) -> serde_json::Value {
    client
        .get(format!("{base}/panes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn test_http_auth_is_optional_when_no_api_key_is_configured() {
    let base = start_test_server().await;
    let response = reqwest::Client::new()
        .get(format!("{base}/panes"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_http_auth_accepts_bearer_and_x_api_key() {
    let base = start_test_server_with_api_key(Some("secret-token")).await;
    let client = reqwest::Client::new();

    let unauthorized = client.get(format!("{base}/panes")).send().await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let wrong = client
        .get(format!("{base}/panes"))
        .header("x-api-key", "wrong-token")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let bearer = client
        .get(format!("{base}/panes"))
        .bearer_auth("secret-token")
        .send()
        .await
        .unwrap();
    assert_eq!(bearer.status(), StatusCode::OK);

    let x_api_key = client
        .get(format!("{base}/panes"))
        .header("x-api-key", "secret-token")
        .send()
        .await
        .unwrap();
    assert_eq!(x_api_key.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_websocket_auth_requires_api_key_when_configured() {
    use tokio_tungstenite::connect_async;

    let base = start_test_server_with_api_key(Some("socket-secret")).await;
    let ws_base = base.replace("http://", "ws://");

    let error = connect_async(format!("{ws_base}/panes/lifecycle"))
        .await
        .expect_err("unauthenticated websocket connection should fail");
    let response = match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => response,
        other => panic!("expected websocket HTTP error, got {other:?}"),
    };
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let mut request = format!("{ws_base}/panes/lifecycle")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer socket-secret"),
    );

    let (mut ws, _) = connect_async(request).await.unwrap();
    let snapshot = recv_lifecycle_event(&mut ws).await;
    assert_eq!(snapshot["type"].as_str(), Some("snapshot"));
    ws.close(None).await.ok();
}

async fn recv_lifecycle_event(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> serde_json::Value {
    use futures_util::StreamExt;

    loop {
        match ws.next().await {
            Some(Ok(tokio_tungstenite::tungstenite::Message::Text(txt))) => {
                return serde_json::from_str(&txt).unwrap();
            }
            Some(Ok(_)) => {}
            Some(Err(err)) => panic!("websocket error: {err}"),
            None => panic!("websocket closed unexpectedly"),
        }
    }
}

#[tokio::test]
async fn test_echo_hello() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let id = create_pane(&client, &base).await;
    sleep(Duration::from_millis(300)).await;

    send_input(&client, &base, &id, "echo hello\n").await;
    sleep(Duration::from_millis(500)).await;

    let screen = get_screen(&client, &base, &id).await;
    let rows = screen["rows"].as_array().unwrap();
    let content: String = rows
        .iter()
        .filter_map(|r| r.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        content.contains("hello"),
        "Expected 'hello' in screen, got:\n{content}"
    );

    delete_pane(&client, &base, &id).await;
}

#[tokio::test]
async fn test_events_log() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let id = create_pane(&client, &base).await;
    sleep(Duration::from_millis(300)).await;

    send_input(&client, &base, &id, "echo events_test\n").await;
    sleep(Duration::from_millis(500)).await;

    let events: serde_json::Value = client
        .get(format!("{base}/panes/{id}/events?after=0"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let arr = events.as_array().unwrap();
    assert!(!arr.is_empty(), "Expected events in log");

    // Reconstruct output from base64 event data
    let all_data: Vec<u8> = arr
        .iter()
        .flat_map(|e| {
            let b64 = e["data"].as_str().unwrap_or("");
            STANDARD.decode(b64).unwrap_or_default()
        })
        .collect();

    let output = String::from_utf8_lossy(&all_data);
    assert!(
        output.contains("events_test"),
        "Expected 'events_test' in event log, got: {output:?}"
    );

    delete_pane(&client, &base, &id).await;
}

#[tokio::test]
async fn test_websocket_stream() {
    use futures_util::StreamExt;
    use tokio_tungstenite::connect_async;

    let base = start_test_server().await;
    let ws_base = base.replace("http://", "ws://");
    let client = reqwest::Client::new();

    let id = create_pane(&client, &base).await;
    sleep(Duration::from_millis(300)).await;

    let ws_url = format!("{ws_base}/panes/{id}/stream");
    let (mut ws, _) = connect_async(&ws_url).await.unwrap();

    // Send input so we get an event
    send_input(&client, &base, &id, "echo ws_test\n").await;
    sleep(Duration::from_millis(500)).await;

    // Collect messages with a short timeout
    let mut received_data: Vec<u8> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);

    loop {
        let timeout = tokio::time::sleep_until(deadline);
        tokio::select! {
            msg = ws.next() => {
                match msg {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(txt))) => {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&txt) {
                            if let Some(b64) = parsed["data"].as_str() {
                                if let Ok(bytes) = STANDARD.decode(b64) {
                                    received_data.extend_from_slice(&bytes);
                                }
                            }
                        }
                    }
                    Some(Ok(_)) => {}
                    _ => break,
                }
            }
            _ = timeout => break,
        }
        if String::from_utf8_lossy(&received_data).contains("ws_test") {
            break;
        }
    }

    let output = String::from_utf8_lossy(&received_data);
    assert!(
        output.contains("ws_test"),
        "Expected 'ws_test' in WS stream, got: {output:?}"
    );

    ws.close(None).await.ok();
    delete_pane(&client, &base, &id).await;
}

#[tokio::test]
async fn test_list_and_delete() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let id = create_pane(&client, &base).await;

    let list = list_panes(&client, &base).await;

    let arr = list.as_array().unwrap();
    assert!(
        arr.iter().any(|p| p["id"].as_str() == Some(&id)),
        "Created pane should appear in list"
    );

    delete_pane(&client, &base, &id).await;

    let list2 = list_panes(&client, &base).await;

    let arr2 = list2.as_array().unwrap();
    assert!(
        !arr2.iter().any(|p| p["id"].as_str() == Some(&id)),
        "Deleted pane should not appear in list"
    );
}

#[tokio::test]
async fn test_config_updates_defaults_for_future_panes_only() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let initial = get_config(&client, &base).await;
    assert_eq!(initial["default_cols"].as_u64(), Some(220));
    assert_eq!(initial["default_rows"].as_u64(), Some(50));

    let before = create_pane_with_body_response(&client, &base, serde_json::json!({})).await;
    let before_id = before["id"].as_str().unwrap().to_string();
    assert_eq!(before["cols"].as_u64(), Some(220));
    assert_eq!(before["rows"].as_u64(), Some(50));

    let updated: serde_json::Value = client
        .patch(format!("{base}/config"))
        .json(&serde_json::json!({ "default_cols": 100, "default_rows": 30 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["default_cols"].as_u64(), Some(100));
    assert_eq!(updated["default_rows"].as_u64(), Some(30));

    let after = create_pane_with_body_response(&client, &base, serde_json::json!({})).await;
    let after_id = after["id"].as_str().unwrap().to_string();
    assert_eq!(after["cols"].as_u64(), Some(100));
    assert_eq!(after["rows"].as_u64(), Some(30));

    let list = list_panes(&client, &base).await;
    let panes = list.as_array().unwrap();
    let before_pane = panes
        .iter()
        .find(|pane| pane["id"].as_str() == Some(before_id.as_str()))
        .expect("existing pane should still exist");
    let after_pane = panes
        .iter()
        .find(|pane| pane["id"].as_str() == Some(after_id.as_str()))
        .expect("new pane should exist");

    assert_eq!(before_pane["cols"].as_u64(), Some(220));
    assert_eq!(before_pane["rows"].as_u64(), Some(50));
    assert_eq!(after_pane["cols"].as_u64(), Some(100));
    assert_eq!(after_pane["rows"].as_u64(), Some(30));

    delete_pane(&client, &base, &before_id).await;
    delete_pane(&client, &base, &after_id).await;
}

#[tokio::test]
async fn test_config_apply_to_existing_resizes_running_panes() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let first = create_pane_with_body_response(&client, &base, serde_json::json!({})).await;
    let second = create_pane_with_body_response(
        &client,
        &base,
        serde_json::json!({ "cols": 90, "rows": 25 }),
    )
    .await;
    let first_id = first["id"].as_str().unwrap().to_string();
    let second_id = second["id"].as_str().unwrap().to_string();

    let response = client
        .patch(format!("{base}/config"))
        .json(&serde_json::json!({
            "default_cols": 132,
            "default_rows": 41,
            "apply_to_existing": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let list = list_panes(&client, &base).await;
    for id in [&first_id, &second_id] {
        let pane = list
            .as_array()
            .unwrap()
            .iter()
            .find(|pane| pane["id"].as_str() == Some(id.as_str()))
            .expect("pane should appear in list");
        assert_eq!(pane["cols"].as_u64(), Some(132));
        assert_eq!(pane["rows"].as_u64(), Some(41));

        let screen = get_screen(&client, &base, id).await;
        assert_eq!(screen["num_cols"].as_u64(), Some(132));
        assert_eq!(screen["num_rows"].as_u64(), Some(41));
    }

    delete_pane(&client, &base, &first_id).await;
    delete_pane(&client, &base, &second_id).await;
}

#[tokio::test]
async fn test_resize_updates_list_dimensions() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();
    let id = create_pane(&client, &base).await;

    let response = client
        .post(format!("{base}/panes/{id}/resize"))
        .json(&serde_json::json!({ "cols": 132, "rows": 41 }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let list = list_panes(&client, &base).await;
    let pane = list
        .as_array()
        .unwrap()
        .iter()
        .find(|pane| pane["id"].as_str() == Some(id.as_str()))
        .expect("pane should appear in list");
    assert_eq!(pane["cols"].as_u64(), Some(132));
    assert_eq!(pane["rows"].as_u64(), Some(41));

    delete_pane(&client, &base, &id).await;
}

#[tokio::test]
async fn test_list_reports_idle_seconds_and_resets_on_input() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let id = create_pane(&client, &base).await;
    sleep(Duration::from_millis(300)).await;

    send_input(&client, &base, &id, "stty -echo\n").await;
    sleep(Duration::from_millis(500)).await;
    sleep(Duration::from_millis(1100)).await;

    let list_before_input = list_panes(&client, &base).await;
    let pane_before_input = list_before_input
        .as_array()
        .unwrap()
        .iter()
        .find(|pane| pane["id"].as_str() == Some(id.as_str()))
        .expect("pane should appear in list before input");
    let idle_before_input = pane_before_input["idle_seconds"]
        .as_u64()
        .expect("idle_seconds should be returned as an integer");
    assert!(
        idle_before_input >= 1,
        "idle_seconds should increase while pane is idle, got {idle_before_input}"
    );

    send_input(&client, &base, &id, "x").await;
    sleep(Duration::from_millis(100)).await;

    let list_after_input = list_panes(&client, &base).await;
    let pane_after_input = list_after_input
        .as_array()
        .unwrap()
        .iter()
        .find(|pane| pane["id"].as_str() == Some(id.as_str()))
        .expect("pane should appear in list after input");
    let idle_after_input = pane_after_input["idle_seconds"]
        .as_u64()
        .expect("idle_seconds should be returned as an integer");
    assert_eq!(
        idle_after_input, 0,
        "idle_seconds should reset immediately after input activity, got {idle_after_input}"
    );

    delete_pane(&client, &base, &id).await;
}

#[tokio::test]
async fn test_pane_lifecycle_stream() {
    use tokio_tungstenite::connect_async;

    let base = start_test_server().await;
    let ws_base = base.replace("http://", "ws://");
    let client = reqwest::Client::new();

    let (mut ws, _) = connect_async(format!("{ws_base}/panes/lifecycle"))
        .await
        .unwrap();

    let snapshot = recv_lifecycle_event(&mut ws).await;
    assert_eq!(snapshot["type"].as_str(), Some("snapshot"));

    let resp: serde_json::Value = client
        .post(format!("{base}/panes"))
        .json(&serde_json::json!({ "cols": 80, "rows": 24, "name": "streamed" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let id = resp["id"].as_str().unwrap().to_string();

    let created = recv_lifecycle_event(&mut ws).await;
    assert_eq!(created["type"].as_str(), Some("created"));
    assert_eq!(created["pane"]["id"].as_str(), Some(id.as_str()));
    assert_eq!(created["pane"]["name"].as_str(), Some("streamed"));

    delete_pane(&client, &base, &id).await;

    let deleted = recv_lifecycle_event(&mut ws).await;
    assert_eq!(deleted["type"].as_str(), Some("deleted"));
    assert_eq!(deleted["id"].as_str(), Some(id.as_str()));

    ws.close(None).await.ok();
}

#[tokio::test]
async fn test_pane_name() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    // Create a named pane
    let resp: serde_json::Value = client
        .post(format!("{base}/panes"))
        .json(&serde_json::json!({ "cols": 80, "rows": 24, "name": "my-shell" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let id = resp["id"].as_str().unwrap().to_string();
    assert_eq!(
        resp["name"].as_str(),
        Some("my-shell"),
        "name should be returned on create"
    );

    // Verify name appears in list
    let list: serde_json::Value = client
        .get(format!("{base}/panes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let pane = list
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"].as_str() == Some(&id))
        .expect("pane should appear in list");

    assert_eq!(
        pane["name"].as_str(),
        Some("my-shell"),
        "name should appear in list"
    );

    delete_pane(&client, &base, &id).await;
}

#[tokio::test]
async fn test_named_pane_routes_accept_name() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();
    let pane_name = "named-route";

    let id = create_pane_with_name(&client, &base, pane_name).await;
    sleep(Duration::from_millis(300)).await;

    let input_response = client
        .post(format!("{base}/panes/{pane_name}/input"))
        .json(&serde_json::json!({ "data": STANDARD.encode("echo via_name\n".as_bytes()) }))
        .send()
        .await
        .unwrap();
    assert_eq!(input_response.status(), StatusCode::NO_CONTENT);

    sleep(Duration::from_millis(500)).await;

    let events: serde_json::Value = client
        .get(format!("{base}/panes/{pane_name}/events?after=0"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let event_output = String::from_utf8_lossy(
        &events
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|event| {
                STANDARD
                    .decode(event["data"].as_str().unwrap_or_default())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>(),
    )
    .to_string();
    assert!(
        event_output.contains("via_name"),
        "expected name-routed events to include pane output, got: {event_output:?}"
    );

    let screen: serde_json::Value = client
        .get(format!("{base}/panes/{pane_name}/screen"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let screen_content = screen["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        screen_content.contains("via_name"),
        "expected name-routed screen to include pane output, got:\n{screen_content}"
    );

    let delete_response = client
        .delete(format!("{base}/panes/{pane_name}"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    let list: serde_json::Value = client
        .get(format!("{base}/panes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !list
            .as_array()
            .unwrap()
            .iter()
            .any(|pane| pane["id"].as_str() == Some(id.as_str())),
        "deleted pane should be gone after name-routed delete"
    );
}

#[tokio::test]
async fn test_named_pane_routes_accept_uuid_shaped_name() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();
    let pane_name = "123e4567-e89b-12d3-a456-426614174000";

    create_pane_with_name(&client, &base, pane_name).await;
    sleep(Duration::from_millis(300)).await;

    let input_response = client
        .post(format!("{base}/panes/{pane_name}/input"))
        .json(&serde_json::json!({ "data": STANDARD.encode("echo uuid_name\n".as_bytes()) }))
        .send()
        .await
        .unwrap();
    assert_eq!(input_response.status(), StatusCode::NO_CONTENT);

    sleep(Duration::from_millis(500)).await;

    let screen: serde_json::Value = client
        .get(format!("{base}/panes/{pane_name}/screen"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let screen_content = screen["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        screen_content.contains("uuid_name"),
        "expected UUID-shaped pane name to resolve via name routing, got:\n{screen_content}"
    );

    delete_pane(&client, &base, pane_name).await;
}

#[tokio::test]
async fn test_named_pane_routes_error_on_duplicate_names() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();
    let pane_name = "duplicate-name";

    let first_id = create_pane_with_name(&client, &base, pane_name).await;
    let second_id = create_pane_with_name(&client, &base, pane_name).await;

    let response = client
        .get(format!("{base}/panes/{pane_name}/screen"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(
        body["error"].as_str(),
        Some("multiple panes named duplicate-name found")
    );

    delete_pane(&client, &base, &first_id).await;
    delete_pane(&client, &base, &second_id).await;
}

#[tokio::test]
async fn test_exit_terminates_pane() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let id = create_pane(&client, &base).await;
    sleep(Duration::from_millis(300)).await;

    send_input(&client, &base, &id, "exit\n").await;
    sleep(Duration::from_millis(800)).await;

    let list: serde_json::Value = client
        .get(format!("{base}/panes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let pane = list
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"].as_str() == Some(&id))
        .expect("pane should still appear in list after exit");

    assert!(
        pane["terminated"].as_bool() == Some(true),
        "pane should be terminated after shell exits, got: {pane:?}"
    );

    delete_pane(&client, &base, &id).await;
}

#[tokio::test]
async fn test_pane_name_optional() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    // Create a pane without a name
    let resp: serde_json::Value = client
        .post(format!("{base}/panes"))
        .json(&serde_json::json!({ "cols": 80, "rows": 24 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let id = resp["id"].as_str().unwrap().to_string();
    assert!(
        resp["name"].is_null(),
        "name should be null when not provided"
    );

    // Verify null name in list
    let list: serde_json::Value = client
        .get(format!("{base}/panes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let pane = list
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"].as_str() == Some(&id))
        .expect("pane should appear in list");

    assert!(
        pane["name"].is_null(),
        "name should be null in list when not set"
    );

    delete_pane(&client, &base, &id).await;
}

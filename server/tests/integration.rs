use base64::{engine::general_purpose::STANDARD, Engine};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::sleep;

async fn start_test_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = terminal_server::state::AppState::new();
    let router = terminal_server::build_router(state);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://127.0.0.1:{}", port)
}

async fn create_pane(client: &reqwest::Client, base: &str) -> String {
    let resp: serde_json::Value = client
        .post(format!("{base}/panes"))
        .json(&serde_json::json!({ "cols": 80, "rows": 24 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    resp["id"].as_str().unwrap().to_string()
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

    let list: serde_json::Value = client
        .get(format!("{base}/panes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let arr = list.as_array().unwrap();
    assert!(
        arr.iter().any(|p| p["id"].as_str() == Some(&id)),
        "Created pane should appear in list"
    );

    delete_pane(&client, &base, &id).await;

    let list2: serde_json::Value = client
        .get(format!("{base}/panes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let arr2 = list2.as_array().unwrap();
    assert!(
        !arr2.iter().any(|p| p["id"].as_str() == Some(&id)),
        "Deleted pane should not appear in list"
    );
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

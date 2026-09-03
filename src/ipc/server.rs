//! IPC server: a tiny HTTP-like JSON server over a Unix socket.
//!
//! API:
//! ```text
//! GET  /v1/node
//! GET  /v1/peers
//! GET  /v1/peers/:id
//! POST /v1/peers
//! DELETE /v1/peers/:id
//! POST /v1/connect/:id
//! POST /v1/disconnect/:id
//! GET  /v1/services
//! GET  /v1/status
//! ```
//!
//! Responses are JSON. Errors use `{"error": "..."}`.

use crate::daemon::Daemon;
use crate::error::Result;
use crate::peer::Peer;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

/// Serialize an error for the IPC JSON response.
fn error_json(e: &crate::error::Error) -> Value {
    json!({"error": e.inner_message()})
}

/// Start the IPC server on the given Unix socket path.
pub async fn serve(daemon: Arc<Daemon>, socket_path: impl AsRef<Path>) -> Result<()> {
    let socket_path = socket_path.as_ref();
    // Remove any stale socket.
    let _ = std::fs::remove_file(socket_path);

    let listener = UnixListener::bind(socket_path)
        .map_err(|e| crate::error::Error::Ipc(format!("bind {}: {}", socket_path.display(), e)))?;

    tracing::info!("IPC server listening at {}", socket_path.display());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let daemon = daemon.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, daemon).await {
                        tracing::warn!("IPC connection error: {}", e);
                    }
                });
            }
            Err(e) => {
                tracing::warn!("IPC accept error: {}", e);
            }
        }
    }
}

/// Handle a single IPC connection.
async fn handle_connection(mut stream: tokio::net::UnixStream, daemon: Arc<Daemon>) -> Result<()> {
    let mut buf = Vec::with_capacity(8192);
    let mut tmp = [0u8; 4096];

    // Read until we have the full headers.
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 65536 {
            return Ok(());
        }
    }

    let request = String::from_utf8_lossy(&buf);
    let (method, path, body) = parse_request(&request);

    let response = route(&daemon, &method, &path, &body).await;

    let response_str = format!(
        "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.0,
        response.1.len(),
        response.1
    );

    stream.write_all(response_str.as_bytes()).await?;
    stream.flush().await?;

    Ok(())
}

/// Parse an HTTP-like request into (method, path, body).
fn parse_request(request: &str) -> (String, String, String) {
    let mut lines = request.split("\r\n");
    let first_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    let method = parts.first().map(|s| s.to_string()).unwrap_or_default();
    let path = parts.get(1).map(|s| s.to_string()).unwrap_or_default();

    // Find body (after \r\n\r\n).
    let body = request.split("\r\n\r\n").nth(1).unwrap_or("").to_string();

    (method, path, body)
}

/// Route a request to the appropriate handler.
async fn route(daemon: &Daemon, method: &str, path: &str, body: &str) -> (u16, String) {
    let (status, value) = match (method, path) {
        ("GET", "/v1/node") => {
            let info = daemon.node_info();
            (200, serde_json::to_value(&info).unwrap_or_default())
        }
        ("GET", "/v1/peers") => {
            let peers = daemon.list_peers().await;
            (200, serde_json::to_value(&peers).unwrap_or_default())
        }
        ("GET", p) if p.starts_with("/v1/peers/") => {
            let id = p.trim_start_matches("/v1/peers/");
            match daemon.get_peer(id).await {
                Some(peer) => (200, serde_json::to_value(&peer).unwrap_or_default()),
                None => (404, json!({"error": format!("peer not found: {}", id)})),
            }
        }
        ("POST", "/v1/peers") => match serde_json::from_str::<Value>(body) {
            Ok(peer_data) => {
                let id = peer_data.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = peer_data.get("name").and_then(|v| v.as_str()).unwrap_or(id);
                let token = peer_data
                    .get("token")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let peer = Peer::new(id, name, token);
                let mut registry = daemon.peer_registry_for_write().await;
                registry.add(peer);
                if let Err(e) = registry.save() {
                    (500, error_json(&e))
                } else {
                    (201, json!({"ok": true}))
                }
            }
            Err(e) => (400, json!({"error": e.to_string()})),
        },
        ("DELETE", p) if p.starts_with("/v1/peers/") => {
            let id = p.trim_start_matches("/v1/peers/");
            let mut registry = daemon.peer_registry_for_write().await;
            if registry.remove(id) {
                if let Err(e) = registry.save() {
                    (500, error_json(&e))
                } else {
                    (200, json!({"ok": true}))
                }
            } else {
                (404, json!({"error": format!("peer not found: {}", id)}))
            }
        }
        ("POST", p) if p.starts_with("/v1/connect/") => {
            let id = p.trim_start_matches("/v1/connect/");
            match daemon.connect_peer(id).await {
                Ok(status) => (200, serde_json::to_value(&status).unwrap_or_default()),
                Err(e) => (500, error_json(&e)),
            }
        }
        ("POST", p) if p.starts_with("/v1/disconnect/") => {
            let id = p.trim_start_matches("/v1/disconnect/");
            match daemon.disconnect_peer(id).await {
                Ok(()) => (200, json!({"ok": true})),
                Err(e) => (500, error_json(&e)),
            }
        }
        ("GET", "/v1/services") => {
            let services = daemon.list_services().await;
            (200, serde_json::to_value(&services).unwrap_or_default())
        }
        ("GET", "/v1/status") => {
            let statuses = daemon.peer_statuses().await;
            (200, serde_json::to_value(&statuses).unwrap_or_default())
        }
        _ => (
            404,
            json!({"error": format!("not found: {} {}", method, path)}),
        ),
    };

    (status, serde_json::to_string(&value).unwrap_or_default())
}

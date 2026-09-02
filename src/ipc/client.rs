//! IPC client: used by CLI commands to talk to the running daemon.
//!
//! The client sends a minimal HTTP-like request over the Unix socket
//! and parses the JSON response body.

use crate::error::{Error, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// Client for the daemon's IPC API.
pub struct IpcClient {
    socket: PathBuf,
}

/// Compute the socket path for a config directory.
fn socket_path(config_dir: &Path) -> PathBuf {
    config_dir.join("tailcat-node.sock")
}

impl IpcClient {
    pub fn new(config_dir: impl AsRef<Path>) -> Self {
        Self {
            socket: socket_path(config_dir.as_ref()),
        }
    }

    /// Send a GET request.
    pub async fn get(&self, path: &str) -> Result<Value> {
        self.request("GET", path, None).await
    }

    /// Send a POST request.
    pub async fn post(&self, path: &str, body: Option<&str>) -> Result<Value> {
        self.request("POST", path, body).await
    }

    /// Send a DELETE request.
    pub async fn delete(&self, path: &str) -> Result<Value> {
        self.request("DELETE", path, None).await
    }

    /// Send a request and parse the JSON response.
    async fn request(&self, method: &str, path: &str, body: Option<&str>) -> Result<Value> {
        if !self.socket.exists() {
            return Err(Error::NotRunning(self.socket.display().to_string()));
        }

        // Connect to the Unix socket.
        let mut stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|e| Error::Ipc(format!("connect: {}", e)))?;

        // Build the HTTP-like request.
        let body_bytes = body.unwrap_or("").as_bytes();
        let request = format!(
            "{} {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            method,
            path,
            body_bytes.len()
        );

        // Send request.
        stream.write_all(request.as_bytes()).await?;
        if let Some(body) = body {
            stream.write_all(body.as_bytes()).await?;
        }
        stream.flush().await?;

        // Read response.
        let mut buf = Vec::with_capacity(8192);
        let mut tmp = [0u8; 4096];
        loop {
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.len() > 65536 {
                return Err(Error::Ipc("response too large".to_string()));
            }
        }

        // Parse the response.
        let response = String::from_utf8_lossy(&buf);

        // Find the body (after \r\n\r\n).
        let body_start = response
            .find("\r\n\r\n")
            .ok_or_else(|| Error::Ipc("malformed response".to_string()))?;
        let body_text = &response[body_start + 4..];

        let value: Value = serde_json::from_str(body_text)?;
        Ok(value)
    }
}

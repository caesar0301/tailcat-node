//! Tailcat network backend abstraction.
//!
//! `tailcat-node` delegates encrypted connectivity (WireGuard,
//! magicsock, DERP, NAT traversal, P2P) to Tailcat. This module
//! exposes a [`Backend`] trait so the daemon is agnostic to whether
//! we invoke the `tailcat` binary or embed the Tailcat library.
//!
//! The daemon should **not implement NAT traversal itself**.
//!
//! When the `tailcat` binary is not on PATH, `build_backend()` returns
//! `None`. The daemon starts in **degraded mode**: peer/service
//! management and IPC work normally, but network operations (connect,
//! disconnect, ping) return a clear error. No mock or stub backend is
//! used — we never fabricate connection state.

use crate::error::{Error, Result};
use crate::peer::{ConnectionPath, PeerState, PeerStatus};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::process::Command;

/// Result of a ping operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingResult {
    pub peer_id: String,
    pub reachable: bool,
    pub path: ConnectionPath,
    pub latency_ms: u32,
}

/// Peer status as reported by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendPeerStatus {
    pub peer_id: String,
    pub state: PeerState,
    pub path: ConnectionPath,
    pub latency_ms: Option<u32>,
}

/// The network backend trait.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Start the backend.
    async fn start(&self) -> Result<()>;

    /// Stop the backend.
    async fn stop(&self) -> Result<()>;

    /// Connect to a peer.
    async fn connect(&self, peer_id: &str, token: &str) -> Result<PeerStatus>;

    /// Disconnect from a peer.
    async fn disconnect(&self, peer_id: &str) -> Result<()>;

    /// Ping a peer.
    async fn ping(&self, peer_id: &str, token: &str) -> Result<PingResult>;
}

/// Process-based backend that invokes the `tailcat` binary.
///
/// This backend shells out to `tailcat` for network operations:
/// - `start`: ensures a default server key exists (`tailcat genkey --key=default`)
/// - `stop`: no-op (tailcat connections are per-invocation, no long-running process to kill)
/// - `connect`: pings the peer's tailcat address to establish a path
/// - `disconnect`: no-op (tailcat connections are ephemeral)
/// - `ping`: runs `tailcat ping <addr>` and parses the pong output
pub struct TailcatProcessBackend {
    /// Path to the `tailcat` binary.
    binary: std::path::PathBuf,
}

impl TailcatProcessBackend {
    /// Create a new process backend for the given binary path.
    pub fn new(binary: std::path::PathBuf) -> Self {
        Self { binary }
    }

    /// Run the tailcat binary with the given args, returning stdout.
    async fn run(&self, args: &[&str]) -> Result<String> {
        let output = Command::new(&self.binary)
            .args(args)
            .output()
            .await
            .map_err(|e| Error::Backend(format!("failed to execute tailcat: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(Error::Backend(format!(
                "tailcat {} failed: {}",
                args.join(" "),
                stderr
                    .trim()
                    .is_empty()
                    .then_some(stdout.trim())
                    .unwrap_or(stderr.trim())
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[async_trait]
impl Backend for TailcatProcessBackend {
    async fn start(&self) -> Result<()> {
        // Ensure a default server key exists. `tailcat genkey --key=default`
        // is idempotent — it won't overwrite an existing key.
        // We check if the key already exists first to avoid noisy logs.
        let list_output = self.run(&["genkey", "--list"]).await.unwrap_or_default();
        if !list_output.contains("default") {
            tracing::info!("Generating default tailcat server key...");
            self.run(&["genkey", "--key=default"]).await?;
        }
        tracing::info!("tailcat backend started (default key ready)");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        // tailcat connections are per-invocation (no long-running daemon
        // process to kill). Nothing to stop.
        tracing::info!("tailcat backend stopped (no-op — tailcat has no persistent process)");
        Ok(())
    }

    async fn connect(&self, peer_id: &str, token: &str) -> Result<PeerStatus> {
        // The peer's tailcat address is stored in the token field.
        // We ping it to establish a path and report connection status.
        let ping_result = self.ping(peer_id, token).await?;
        Ok(ping_result_to_status(&ping_result))
    }

    async fn disconnect(&self, _peer_id: &str) -> Result<()> {
        // tailcat connections are ephemeral (each invocation is a separate
        // connection). There's no persistent connection to tear down.
        Ok(())
    }

    async fn ping(&self, peer_id: &str, token: &str) -> Result<PingResult> {
        // The token field holds the peer's tailcat address (tc-addr).
        // Run: tailcat ping --timeout=10s <tc-addr>
        let output = self
            .run(&["ping", "--timeout=10s", token])
            .await
            .map_err(|e| {
                Error::Backend(format!(
                    "ping to peer {peer_id} failed: {}",
                    e.inner_message()
                ))
            })?;

        // Parse pong output. tailcat ping prints lines like:
        //   "pong in 42.1ms via DERP(sfo)"
        //   "pong in 1.2ms via 203.0.113.7:41641"
        let (reachable, path, latency_ms) = parse_pong_output(&output);

        Ok(PingResult {
            peer_id: peer_id.to_string(),
            reachable,
            path,
            latency_ms,
        })
    }
}

/// Parse tailcat's pong output to extract reachability, path, and latency.
///
/// Pong lines look like:
///   "pong in 42.1ms via DERP(sfo)"
///   "pong in 1.2ms via 203.0.113.7:41641"
fn parse_pong_output(output: &str) -> (bool, ConnectionPath, u32) {
    let mut reachable = false;
    let mut path = ConnectionPath::Unknown;
    let mut latency_ms = 0;

    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with("pong in ") {
            continue;
        }
        // Found a pong — peer is reachable.
        reachable = true;

        // Extract latency: "pong in 42.1ms via ..." → 42
        if let Some(latency_str) = line
            .strip_prefix("pong in ")
            .and_then(|s| s.split("ms").next())
        {
            // Parse as float, round to integer ms
            if let Ok(latency_f) = latency_str.trim().parse::<f64>() {
                latency_ms = latency_f.round() as u32;
            }
        }

        // Determine path: DERP or direct
        if line.contains("via DERP") {
            path = ConnectionPath::Derp;
        } else if line.contains("via ") {
            // "via 203.0.113.7:41641" — direct connection
            path = ConnectionPath::Direct;
        }
        break; // Only need the first pong
    }

    (reachable, path, latency_ms)
}

/// Build the appropriate backend.
///
/// If the `tailcat` binary is found on PATH, return the process-based
/// backend. Otherwise, return `None` — the daemon will run in degraded
/// mode (peer/service management works, but network operations fail
/// with a clear error).
pub fn build_backend() -> Option<Arc<dyn Backend>> {
    match which::which("tailcat") {
        Ok(path) => {
            tracing::info!("Found tailcat binary at {}", path.display());
            Some(Arc::new(TailcatProcessBackend::new(path)))
        }
        Err(_) => None,
    }
}

/// Check whether the `tailcat` binary is available on PATH.
pub fn tailcat_available() -> bool {
    which::which("tailcat").is_ok()
}

/// Convert a [`PingResult`] to a [`PeerStatus`].
pub fn ping_result_to_status(result: &PingResult) -> PeerStatus {
    PeerStatus {
        peer_id: result.peer_id.clone(),
        state: if result.reachable {
            PeerState::Connected
        } else {
            PeerState::Failed
        },
        path: result.path,
        latency_ms: Some(result.latency_ms),
        last_connected: if result.reachable {
            Some(Utc::now())
        } else {
            None
        },
        last_error: if result.reachable {
            None
        } else {
            Some("unreachable".to_string())
        },
    }
}

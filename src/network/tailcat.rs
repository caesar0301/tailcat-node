//! Tailcat network backend abstraction.
//!
//! `tailcat-node` delegates encrypted connectivity (WireGuard,
//! magicsock, DERP, NAT traversal, P2P) to Tailcat. This module
//! exposes a [`Backend`] trait so the daemon is agnostic to whether
//! we invoke the `tailcat` binary or embed the Tailcat library.
//!
//! The daemon should **not implement NAT traversal itself**.
//!
//! Two possible implementations:
//! - A. Invoke `tailcat` binary (easy, stable boundary, good for prototyping)
//! - B. Embed Tailcat library (cleaner but adds Rust↔Go boundary)
//!
//! When the `tailcat` binary is not on PATH, `build_backend()` returns
//! `None`. The daemon starts in **degraded mode**: peer/service
//! management and IPC work normally, but network operations (connect,
//! disconnect, ping) return a clear error. No mock or stub backend is
//! used — we never fabricate connection state.

use crate::error::Result;
use crate::peer::{ConnectionPath, PeerState, PeerStatus};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
            // The process-based backend is not yet implemented.
            // TODO: implement TailcatProcessBackend that invokes the
            // `tailcat` binary for connect/disconnect/ping.
            tracing::warn!("tailcat binary found but process backend not yet implemented — running in degraded mode");
            None
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

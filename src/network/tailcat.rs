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
//! We prototype A first. The mock backend is used when the `tailcat`
//! binary is not available.

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

/// Mock backend used when the `tailcat` binary is not available.
/// Simulates connections with deterministic latency.
pub struct MockBackend;

#[async_trait]
impl Backend for MockBackend {
    async fn start(&self) -> Result<()> {
        tracing::info!("MockBackend started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        tracing::info!("MockBackend stopped");
        Ok(())
    }

    async fn connect(&self, peer_id: &str, _token: &str) -> Result<PeerStatus> {
        tracing::info!("MockBackend connecting to {}", peer_id);
        // Simulate a direct connection with low latency.
        let latency = 5 + (peer_id.len() as u32 % 20);
        Ok(PeerStatus {
            peer_id: peer_id.to_string(),
            state: PeerState::Connected,
            path: ConnectionPath::Direct,
            latency_ms: Some(latency),
            last_connected: Some(Utc::now()),
            last_error: None,
        })
    }

    async fn disconnect(&self, peer_id: &str) -> Result<()> {
        tracing::info!("MockBackend disconnecting from {}", peer_id);
        Ok(())
    }

    async fn ping(&self, peer_id: &str, _token: &str) -> Result<PingResult> {
        let latency = 5 + (peer_id.len() as u32 % 20);
        Ok(PingResult {
            peer_id: peer_id.to_string(),
            reachable: true,
            path: ConnectionPath::Direct,
            latency_ms: latency,
        })
    }
}

/// Build the appropriate backend.
///
/// If the `tailcat` binary is found on PATH, use the process-based
/// backend. Otherwise, fall back to the mock backend.
///
/// Returns `(backend, is_mock)` so the caller can warn the user.
pub fn build_backend() -> (Arc<dyn Backend>, bool) {
    match which::which("tailcat") {
        Ok(path) => {
            tracing::info!("Found tailcat binary at {}", path.display());
            // For now, use the mock backend even if tailcat is found,
            // since the process-based backend is not yet implemented.
            (Arc::new(MockBackend), true)
        }
        Err(_) => {
            (Arc::new(MockBackend), true)
        }
    }
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

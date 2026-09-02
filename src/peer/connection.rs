//! Connection state machine and in-memory connection table.
//!
//! Lifecycle:
//! ```text
//! DISCOVERED -> AVAILABLE -> CONNECTING -> CONNECTED -> IDLE -> DISCONNECTED
//!                                       \-> FAILED
//! ```

use crate::peer::{ConnectionPath, PeerState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Runtime status of a peer connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerStatus {
    pub peer_id: String,
    pub state: PeerState,
    pub path: ConnectionPath,
    pub latency_ms: Option<u32>,
    pub last_connected: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

impl PeerStatus {
    pub fn new(peer_id: &str) -> Self {
        Self {
            peer_id: peer_id.to_string(),
            state: PeerState::Unknown,
            path: ConnectionPath::Unknown,
            latency_ms: None,
            last_connected: None,
            last_error: None,
        }
    }
}

/// In-memory connection table. Runtime state only — never persisted
/// to `peers.toml`.
#[derive(Clone)]
pub struct ConnectionTable {
    inner: Arc<RwLock<HashMap<String, PeerStatus>>>,
}

impl ConnectionTable {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get(&self, peer_id: &str) -> Option<PeerStatus> {
        self.inner.read().await.get(peer_id).cloned()
    }

    pub async fn set(&self, peer_id: &str, status: PeerStatus) {
        self.inner.write().await.insert(peer_id.to_string(), status);
    }

    pub async fn list(&self) -> Vec<PeerStatus> {
        self.inner.read().await.values().cloned().collect()
    }

    pub async fn remove(&self, peer_id: &str) {
        self.inner.write().await.remove(peer_id);
    }
}

impl Default for ConnectionTable {
    fn default() -> Self {
        Self::new()
    }
}

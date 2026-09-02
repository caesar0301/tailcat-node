//! Daemon lifecycle events.
//!
//! Lifecycle:
//! ```text
//! DISCOVERED -> AVAILABLE -> CONNECTING -> CONNECTED -> IDLE -> DISCONNECTED
//!                                       \-> FAILED
//! ```

use serde::{Deserialize, Serialize};

/// A lifecycle event emitted by the daemon. Useful for future
/// controller integration and diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleEvent {
    pub peer_id: String,
    pub event: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub detail: Option<String>,
}

impl LifecycleEvent {
    pub fn new(peer_id: impl Into<String>, event: impl Into<String>) -> Self {
        Self {
            peer_id: peer_id.into(),
            event: event.into(),
            timestamp: chrono::Utc::now(),
            detail: None,
        }
    }
}

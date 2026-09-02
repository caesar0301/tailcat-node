//! Identity data types.

use serde::{Deserialize, Serialize};

/// Full identity bundle: logical node id + Tailcat key material.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    /// Logical node id (e.g. "agent-001").
    pub node_id: String,
    /// Tailcat private key (opaque blob, base64-ish).
    pub private_key: String,
    /// Tailcat public key derived from the private key.
    pub public_key: String,
}

/// Lightweight node identity used for display and IPC responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeIdentity {
    pub node_id: String,
    pub node_name: String,
    pub public_key: String,
}

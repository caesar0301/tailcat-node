//! Peer management.
//!
//! `peers.toml` is desired configuration (which peers we know about).
//! Runtime connection state lives in `state/` and is never persisted
//! back into `peers.toml`.

pub mod connection;
pub mod registry;

pub use connection::{ConnectionTable, PeerStatus};
pub use registry::PeerRegistry;
pub use types::{ConnectionPath, Peer, PeerState};

mod types {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    /// Connection lifecycle state.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "lowercase")]
    pub enum PeerState {
        Disabled,
        Unknown,
        Available,
        Connecting,
        Connected,
        Failed,
    }

    impl std::fmt::Display for PeerState {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                PeerState::Disabled => write!(f, "disabled"),
                PeerState::Unknown => write!(f, "unknown"),
                PeerState::Available => write!(f, "available"),
                PeerState::Connecting => write!(f, "connecting"),
                PeerState::Connected => write!(f, "connected"),
                PeerState::Failed => write!(f, "failed"),
            }
        }
    }

    /// The transport path used for a connection.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "lowercase")]
    pub enum ConnectionPath {
        Direct,
        Derp,
        Unknown,
    }

    impl std::fmt::Display for ConnectionPath {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                ConnectionPath::Direct => write!(f, "direct"),
                ConnectionPath::Derp => write!(f, "derp"),
                ConnectionPath::Unknown => write!(f, "unknown"),
            }
        }
    }

    /// A peer entry in `peers.toml`.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Peer {
        pub id: String,
        pub name: String,
        pub token: String,
        #[serde(default = "default_true")]
        pub enabled: bool,
        #[serde(default)]
        pub metadata: HashMap<String, String>,
    }

    fn default_true() -> bool {
        true
    }

    impl Peer {
        pub fn new(
            id: impl Into<String>,
            name: impl Into<String>,
            token: impl Into<String>,
        ) -> Self {
            Self {
                id: id.into(),
                name: name.into(),
                token: token.into(),
                enabled: true,
                metadata: HashMap::new(),
            }
        }
    }
}

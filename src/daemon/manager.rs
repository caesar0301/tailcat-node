//! Daemon manager: orchestrates identity, peers, services, connections.
//!
//! `tailcat-node` owns node lifecycle, peer management and
//! agent-level semantics. Tailcat owns encrypted connectivity.
//!
//! The daemon holds:
//! - the loaded config
//! - the node identity
//! - a peer registry (desired config)
//! - a service registry
//! - a connection table (runtime state)
//! - a network backend

use crate::config::Config;
use crate::error::{Error, Result};
use crate::identity::Identity;
use crate::network::{build_backend, Backend};
use crate::peer::{ConnectionPath, ConnectionTable, Peer, PeerRegistry, PeerState, PeerStatus};
use crate::service::{Service, ServiceRegistry};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Lightweight node info for IPC responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub node_id: String,
    pub node_name: Option<String>,
    pub public_key: String,
    pub version: String,
    pub backend_available: bool,
}

/// The daemon orchestrates all subsystems.
pub struct Daemon {
    dir: PathBuf,
    pub config: Config,
    identity: Identity,
    peer_registry: RwLock<PeerRegistry>,
    service_registry: RwLock<ServiceRegistry>,
    connection_table: ConnectionTable,
    backend: Option<Arc<dyn Backend>>,
}

impl Daemon {
    pub fn new(dir: PathBuf, config: Config, identity: Identity) -> Result<Self> {
        let mut peer_registry = PeerRegistry::new(dir.clone());
        peer_registry.load()?;

        let mut service_registry = ServiceRegistry::new(dir.clone());
        service_registry.load()?;

        Ok(Self {
            dir,
            config,
            identity,
            peer_registry: RwLock::new(peer_registry),
            service_registry: RwLock::new(service_registry),
            connection_table: ConnectionTable::new(),
            backend: build_backend(),
        })
    }

    /// Return the config directory.
    pub fn config_dir(&self) -> PathBuf {
        self.dir.clone()
    }

    pub fn node_info(&self) -> NodeInfo {
        NodeInfo {
            node_id: self.identity.node_id.clone(),
            node_name: self.config.node.name.clone(),
            public_key: self.identity.public_key.clone(),
            version: crate::VERSION.to_string(),
            backend_available: !self.is_degraded(),
        }
    }

    pub async fn list_peers(&self) -> Vec<Peer> {
        self.peer_registry.read().await.list().to_vec()
    }

    pub async fn get_peer(&self, peer_id: &str) -> Option<Peer> {
        self.peer_registry.read().await.get(peer_id).cloned()
    }

    pub async fn list_services(&self) -> Vec<Service> {
        self.service_registry.read().await.list().to_vec()
    }

    /// Get a write lock on the peer registry (for IPC mutations).
    pub async fn peer_registry_for_write(&self) -> tokio::sync::RwLockWriteGuard<'_, PeerRegistry> {
        self.peer_registry.write().await
    }

    /// Get a write lock on the service registry (for IPC mutations).
    pub async fn service_registry_for_write(
        &self,
    ) -> tokio::sync::RwLockWriteGuard<'_, ServiceRegistry> {
        self.service_registry.write().await
    }

    /// Start the network backend.
    pub async fn backend_start(&self) -> Result<()> {
        match &self.backend {
            Some(b) => b.start().await,
            None => Ok(()), // Degraded mode — nothing to start.
        }
    }

    /// Stop the network backend.
    pub async fn backend_stop(&self) -> Result<()> {
        match &self.backend {
            Some(b) => b.stop().await,
            None => Ok(()),
        }
    }

    /// Whether the daemon is running in degraded mode (no tailcat backend).
    pub fn is_degraded(&self) -> bool {
        self.backend.is_none()
    }

    /// Connect to a peer. Uses lazy connections: only connects when
    /// explicitly requested.
    pub async fn connect_peer(&self, peer_id: &str) -> Result<PeerStatus> {
        let registry = self.peer_registry.read().await;
        let peer = registry
            .get(peer_id)
            .ok_or_else(|| Error::PeerNotFound(peer_id.to_string()))?;
        if !peer.enabled {
            return Err(Error::Daemon(format!("peer {} is disabled", peer_id)));
        }
        let token = peer.token.clone();
        let peer_id = peer.id.clone();
        drop(registry);

        // Set state to connecting.
        let connecting_status = PeerStatus {
            peer_id: peer_id.clone(),
            state: PeerState::Connecting,
            path: ConnectionPath::Unknown,
            latency_ms: None,
            last_connected: None,
            last_error: None,
        };
        self.connection_table.set(&peer_id, connecting_status).await;

        // Connect via backend.
        let result = match &self.backend {
            Some(b) => b.connect(&peer_id, &token).await,
            None => Err(Error::Backend(
                "tailcat binary not found — daemon is running in degraded mode. Install tailcat to enable network operations.".to_string(),
            )),
        };

        match result {
            Ok(status) => {
                self.connection_table.set(&peer_id, status.clone()).await;
                Ok(status)
            }
            Err(e) => {
                let failed_status = PeerStatus {
                    peer_id: peer_id.clone(),
                    state: PeerState::Failed,
                    path: ConnectionPath::Unknown,
                    latency_ms: None,
                    last_connected: None,
                    last_error: Some(e.to_string()),
                };
                self.connection_table.set(&peer_id, failed_status).await;
                Err(e)
            }
        }
    }

    /// Disconnect from a peer.
    pub async fn disconnect_peer(&self, peer_id: &str) -> Result<()> {
        match &self.backend {
            Some(b) => b.disconnect(peer_id).await?,
            None => return Err(Error::Backend(
                "tailcat binary not found — daemon is running in degraded mode. Install tailcat to enable network operations.".to_string(),
            )),
        }
        self.connection_table
            .set(
                peer_id,
                PeerStatus {
                    peer_id: peer_id.to_string(),
                    state: PeerState::Available,
                    path: ConnectionPath::Unknown,
                    latency_ms: None,
                    last_connected: self
                        .connection_table
                        .get(peer_id)
                        .await
                        .and_then(|s| s.last_connected),
                    last_error: None,
                },
            )
            .await;
        Ok(())
    }

    /// Ping a peer.
    pub async fn ping_peer(&self, peer_id: &str) -> Result<crate::network::PingResult> {
        let registry = self.peer_registry.read().await;
        let peer = registry
            .get(peer_id)
            .ok_or_else(|| Error::PeerNotFound(peer_id.to_string()))?;
        let token = peer.token.clone();
        let peer_id = peer.id.clone();
        drop(registry);

        match &self.backend {
            Some(b) => b.ping(&peer_id, &token).await,
            None => Err(Error::Backend(
                "tailcat binary not found — daemon is running in degraded mode. Install tailcat to enable network operations.".to_string(),
            )),
        }
    }

    /// Get the status of a peer.
    pub async fn peer_status(&self, peer_id: &str) -> Option<PeerStatus> {
        self.connection_table.get(peer_id).await
    }

    /// List all peer statuses.
    pub async fn peer_statuses(&self) -> Vec<PeerStatus> {
        self.connection_table.list().await
    }
}

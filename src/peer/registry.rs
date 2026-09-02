//! Peer registry: manages `peers.toml`.
//!
//! `peers.toml` is desired configuration (which peers we know about).
//! Runtime connection state lives in `state/` and is never persisted
//! back into `peers.toml`.

use super::Peer;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Root structure for `peers.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeersFile {
    pub version: u32,
    #[serde(default)]
    pub peers: Vec<Peer>,
}

/// Manages peer configuration on disk.
pub struct PeerRegistry {
    dir: PathBuf,
    peers: Vec<Peer>,
}

impl PeerRegistry {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            dir: config_dir,
            peers: Vec::new(),
        }
    }

    /// Load peers from `peers.toml`.
    pub fn load(&mut self) -> Result<()> {
        let path = self.dir.join("peers.toml");
        if !path.exists() {
            self.peers = Vec::new();
            return Ok(());
        }
        let text = std::fs::read_to_string(&path)?;
        let file: PeersFile = toml::from_str(&text)?;
        self.peers = file.peers;
        Ok(())
    }

    /// Save peers to `peers.toml`.
    pub fn save(&self) -> Result<()> {
        let path = self.dir.join("peers.toml");
        let file = PeersFile {
            version: crate::CONFIG_VERSION,
            peers: self.peers.clone(),
        };
        let text = toml::to_string_pretty(&file)?;
        std::fs::write(&path, text)?;
        Ok(())
    }

    /// List all peers.
    pub fn list(&self) -> &[Peer] {
        &self.peers
    }

    /// Get a peer by ID.
    pub fn get(&self, id: &str) -> Option<&Peer> {
        self.peers.iter().find(|p| p.id == id)
    }

    /// Add a peer. If a peer with the same ID exists, replace it.
    pub fn add(&mut self, peer: Peer) {
        self.peers.retain(|p| p.id != peer.id);
        self.peers.push(peer);
    }

    /// Remove a peer by ID. Returns true if removed.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.peers.len();
        self.peers.retain(|p| p.id != id);
        self.peers.len() != before
    }

    /// Enable a peer. Returns true if found.
    pub fn enable(&mut self, id: &str) -> bool {
        if let Some(peer) = self.peers.iter_mut().find(|p| p.id == id) {
            peer.enabled = true;
            true
        } else {
            false
        }
    }

    /// Disable a peer. Returns true if found.
    pub fn disable(&mut self, id: &str) -> bool {
        if let Some(peer) = self.peers.iter_mut().find(|p| p.id == id) {
            peer.enabled = false;
            true
        } else {
            false
        }
    }
}

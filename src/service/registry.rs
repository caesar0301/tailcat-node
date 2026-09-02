//! Service registry: manages `services.toml`.
//!
//! `services.toml` maps logical service names (e.g. "acp", "agent",
//! "ssh") to local ports and protocols, so a peer can resolve
//! `agent-002/acp` to a concrete connection without knowing tokens,
//! ports, or Tailcat commands.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A service entry in `services.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    pub port: u16,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

fn default_protocol() -> String {
    "http".to_string()
}

impl Service {
    pub fn new(name: impl Into<String>, port: u16, protocol: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            port,
            protocol: protocol.into(),
        }
    }
}

/// Root structure for `services.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicesFile {
    pub version: u32,
    #[serde(default)]
    pub services: Vec<Service>,
}

/// Manages service configuration on disk.
pub struct ServiceRegistry {
    dir: PathBuf,
    services: Vec<Service>,
}

impl ServiceRegistry {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            dir: config_dir,
            services: Vec::new(),
        }
    }

    /// Load services from `services.toml`.
    pub fn load(&mut self) -> Result<()> {
        let path = self.dir.join("services.toml");
        if !path.exists() {
            self.services = Vec::new();
            return Ok(());
        }
        let text = std::fs::read_to_string(&path)?;
        let file: ServicesFile = toml::from_str(&text)?;
        self.services = file.services;
        Ok(())
    }

    /// Save services to `services.toml`.
    pub fn save(&self) -> Result<()> {
        let path = self.dir.join("services.toml");
        let file = ServicesFile {
            version: crate::CONFIG_VERSION,
            services: self.services.clone(),
        };
        let text = toml::to_string_pretty(&file)?;
        std::fs::write(&path, text)?;
        Ok(())
    }

    /// List all services.
    pub fn list(&self) -> &[Service] {
        &self.services
    }

    /// Add a service. If a service with the same name exists, replace it.
    pub fn add(&mut self, service: Service) {
        self.services.retain(|s| s.name != service.name);
        self.services.push(service);
    }

    /// Remove a service by name. Returns true if removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.services.len();
        self.services.retain(|s| s.name != name);
        self.services.len() != before
    }
}

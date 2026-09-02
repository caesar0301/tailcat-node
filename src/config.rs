//! Configuration: global daemon configuration (`config.toml`).
//!
//! `config.toml` describes **this node**. `peers.toml` describes
//! **other nodes**. Don't put peer tokens here.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Root structure for `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub node: NodeConfig,
    pub daemon: DaemonConfig,
    pub network: NetworkConfig,
    #[serde(default)]
    pub security: SecurityConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub listen_port: u16,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_network_mode")]
    pub mode: String,
    #[serde(default)]
    pub derp: DerpConfig,
}

fn default_network_mode() -> String {
    "lazy".to_string()
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            mode: default_network_mode(),
            derp: DerpConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerpConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for DerpConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_true")]
    pub require_peer_auth: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            require_peer_auth: true,
        }
    }
}

impl Config {
    /// Load config from `config.toml` in the given directory.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join("config.toml");
        let text = std::fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&text)?;
        Ok(config)
    }

    /// Save config to `config.toml` in the given directory.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = dir.join("config.toml");
        let text = toml::to_string_pretty(self)?;
        std::fs::write(&path, text)?;
        Ok(())
    }

    /// Create a default config for a node.
    pub fn default_for(node_id: &str, node_name: Option<&str>) -> Self {
        Self {
            version: crate::CONFIG_VERSION,
            node: NodeConfig {
                id: node_id.to_string(),
                name: node_name.map(|s| s.to_string()),
            },
            daemon: DaemonConfig {
                listen_port: 4242,
                logging: LoggingConfig::default(),
            },
            network: NetworkConfig::default(),
            security: SecurityConfig::default(),
        }
    }
}

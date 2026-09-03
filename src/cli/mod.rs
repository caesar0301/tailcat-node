//! CLI: hierarchical subcommands.
//!
//! ```text
//! tailcat-node status | identity | version
//! tailcat-node peer list | add | remove | show | enable | disable
//! tailcat-node connect | disconnect | ping
//! tailcat-node service list | add | remove
//! tailcat-node doctor | logs | install
//! tailcat-node init | start | stop | token
//! ```

pub mod commands;
pub mod format;

pub use commands::run;

use clap::{Parser, Subcommand};

/// tailcat-node: a small cross-platform daemon around Tailcat.
#[derive(Parser, Debug)]
#[command(name = "tailcat-node", version, about)]
pub struct Cli {
    /// Override the config directory (defaults to ~/.config/tailcat-node).
    #[arg(long, env = "TAILCAT_NODE_CONFIG_DIR")]
    pub config_dir: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Initialize the node (generate config, identity, empty peers/services).
    Init {
        /// Node name.
        #[arg(long)]
        name: Option<String>,
        /// Node ID (defaults to agent-<random>).
        #[arg(long)]
        id: Option<String>,
        /// Overwrite existing config.
        #[arg(long)]
        force: bool,
    },
    /// Start the daemon.
    Start {
        /// Run in foreground (internal use: the parent process spawns a
        /// child with this flag, then exits).
        #[arg(long, hide = true)]
        foreground: bool,
    },
    /// Stop the daemon.
    Stop,
    /// Show node status.
    Status,
    /// Show node identity.
    Identity,
    /// Show version.
    Version,
    /// Print a join token for this node.
    Token,
    /// Peer management.
    Peer {
        #[command(subcommand)]
        command: PeerCommand,
    },
    /// Connect to a peer.
    Connect { id: String },
    /// Disconnect from a peer.
    Disconnect { id: String },
    /// Ping a peer.
    Ping { id: String },
    /// Service management.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// Install the tailcat binary (the networking substrate).
    Install {
        /// Force reinstall even if tailcat is already installed.
        #[arg(long)]
        force: bool,
        /// Force a specific install method (brew, go, nix, aur, binary).
        #[arg(long)]
        method: Option<String>,
    },
    /// Run diagnostics.
    Doctor,
    /// Show recent logs.
    Logs,
}

#[derive(Subcommand, Debug)]
pub enum PeerCommand {
    /// List configured peers.
    List,
    /// Add a peer.
    Add {
        /// Peer ID.
        id: String,
        /// Peer token.
        token: String,
        /// Peer name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Remove a peer.
    Remove { id: String },
    /// Show details of a peer.
    Show { id: String },
    /// Enable a peer.
    Enable { id: String },
    /// Disable a peer.
    Disable { id: String },
}

#[derive(Subcommand, Debug)]
pub enum ServiceCommand {
    /// List configured services.
    List,
    /// Add a service.
    Add {
        /// Service name.
        name: String,
        /// Service port.
        port: u16,
        /// Service protocol (http, acp, ssh, ...).
        #[arg(default_value = "http")]
        protocol: String,
    },
    /// Remove a service.
    Remove { name: String },
}

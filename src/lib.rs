//! tailcat-node: a small cross-platform daemon around Tailcat.
//!
//! `tailcat-node` owns node lifecycle, peer management and agent-level
//! semantics. Tailcat owns encrypted connectivity.
//!
//! Design principle: remain extremely thin. Let Tailcat do networking,
//! let the Agent Runtime do agent semantics, and make `tailcat-node`
//! the small bridge between the two.

pub mod cli;
pub mod config;
pub mod daemon;
pub mod error;
pub mod identity;
pub mod ipc;
pub mod network;
pub mod peer;
pub mod service;

pub use error::{Error, Result};

/// Current config schema version.
pub const CONFIG_VERSION: u32 = 1;

/// Daemon version string.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

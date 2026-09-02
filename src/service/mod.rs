//! Service registry.
//!
//! This is where `tailcat-node` becomes useful to the Agent Runtime.
//! `services.toml` maps logical service names (e.g. "acp", "agent",
//! "ssh") to local ports and protocols, so a peer can resolve
//! `agent-002/acp` to a concrete connection without knowing tokens,
//! ports, or Tailcat commands.

pub mod registry;

pub use registry::{Service, ServiceRegistry};
